//! Re-authentication gating for the encrypted history archive: on macOS,
//! require Touch ID (or the device password as fallback) before opening the
//! Scrollback History panel, so someone with hands on an already-unlocked
//! machine can't casually browse persisted command history without a second
//! check. Off macOS this is a no-op (see [`authenticate`]'s non-macOS arm).
//!
//! Two policies compose (see `Settings::history_reauth_interval_minutes`):
//! a fresh check is always required once per app session, plus, if
//! configured, again after an idle interval within a long-running session.

use std::time::{Duration, Instant};

/// Whether a re-auth prompt is due before granting access, given when the
/// last successful auth happened (`None` = never this session) and the
/// configured idle interval (`None` = no interval beyond the once-per-session
/// gate). Pure and platform-independent — kept separate from the actual
/// native prompt so the policy itself is unit-testable without a macOS
/// device.
pub fn is_due(last_auth: Option<Instant>, interval_minutes: Option<u32>, now: Instant) -> bool {
    let Some(last) = last_auth else {
        return true;
    };
    let Some(minutes) = interval_minutes else {
        return false;
    };
    now.saturating_duration_since(last) >= Duration::from_secs(u64::from(minutes) * 60)
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    /// The actual LocalAuthentication interaction — synchronous, and
    /// confined entirely to whichever thread calls it. Apple documents
    /// `LAContext` policy evaluation as safe off the main thread, so
    /// [`authenticate`] runs this on a dedicated thread rather than iced's
    /// own executor, blocking that thread on the OS's asynchronous
    /// completion callback without holding up anything else.
    fn authenticate_blocking(reason: &str) -> bool {
        // SAFETY: `LAContext` is used exactly as Apple's documentation
        // describes — constructed and evaluated off the main thread. The
        // reply block is kept alive (as the local `reply` binding) until
        // it's guaranteed to have already fired, via the blocking `recv()`
        // below happening after the call that hands the OS the block.
        unsafe {
            let ctx = LAContext::new();
            let policy = LAPolicy::DeviceOwnerAuthentication;
            if let Err(e) = ctx.canEvaluatePolicy_error(policy) {
                // No usable authentication method is configured on this
                // machine at all (e.g. no login password set) — fail closed
                // rather than prompting into a guaranteed error. Warned, so
                // "the panel silently won't open" is diagnosable from logs.
                tracing::warn!("history reauth: cannot evaluate device-owner policy: {e:?}");
                return false;
            }

            tracing::info!("history reauth: showing the Touch ID / device password prompt");
            let (tx, rx) = std::sync::mpsc::channel::<(bool, Option<String>)>();
            let ns_reason = NSString::from_str(reason);
            let reply = block2::RcBlock::new(move |success: Bool, error: *mut NSError| {
                let detail = if error.is_null() {
                    None
                } else {
                    Some((*error).localizedDescription().to_string())
                };
                let _ = tx.send((success.as_bool(), detail));
            });
            ctx.evaluatePolicy_localizedReason_reply(policy, &ns_reason, &reply);
            match rx.recv() {
                Ok((true, _)) => {
                    tracing::info!("history reauth: authenticated");
                    true
                }
                Ok((false, detail)) => {
                    // Warn (not debug): includes the OS's own reason — a user
                    // cancel, but also the silent-failure cases (no UI access,
                    // policy unavailable) that otherwise look like "nothing
                    // happened".
                    tracing::warn!(
                        "history reauth: authentication failed or was cancelled: {}",
                        detail.as_deref().unwrap_or("no error detail")
                    );
                    false
                }
                Err(_) => false,
            }
        }
    }

    /// Prompt for Touch ID (or the device password as fallback), off the
    /// main thread so a slow or ignored prompt never stalls the UI. Being an
    /// `async fn`, nothing (including the prompt thread) starts until the
    /// future is first polled — constructing and then dropping it (a task
    /// the runtime never executes) must not flash a prompt at the user.
    pub async fn authenticate(reason: String) -> bool {
        let (tx, rx) = futures_channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(authenticate_blocking(&reason));
        });
        rx.await.unwrap_or(false)
    }
}

#[cfg(target_os = "macos")]
pub use macos::authenticate;

/// Off macOS, re-auth gating is a no-op — grant access immediately. See the
/// module doc; this is a deliberate macOS-first scoping, not an oversight.
#[cfg(not(target_os = "macos"))]
pub async fn authenticate(_reason: String) -> bool {
    true
}

#[cfg(test)]
#[path = "reauth_tests.rs"]
mod tests;
