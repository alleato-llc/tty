use super::*;

#[test]
fn snippet_emits_both_osc133_marks() {
    // The pasteable snippet must define the C (start) and D (finish) hooks.
    assert!(ZSH_SNIPPET.contains("133;C"));
    assert!(ZSH_SNIPPET.contains("133;D"));
    assert!(ZSH_SNIPPET.contains("add-zsh-hook preexec"));
    assert!(ZSH_SNIPPET.contains("add-zsh-hook precmd"));
}

#[test]
fn snippet_captures_env_gated_on_the_flag() {
    // The precmd also dumps env for the Env view — but only while the `.on` flag is
    // present, so it's a no-op stat when the view is closed.
    assert!(ZSH_SNIPPET.contains("_tty_capture_env"));
    assert!(ZSH_SNIPPET.contains("$TTY_ENV_FILE"));
    assert!(ZSH_SNIPPET.contains("${TTY_ENV_FILE}.on"));
    assert!(ZSH_SNIPPET.contains("env >"));
}

#[test]
fn env_channel_path_is_a_fresh_user_only_dir() {
    let a = env_channel_path();
    let b = env_channel_path();
    if let (Some(a), Some(b)) = (&a, &b) {
        assert_ne!(a, b, "each session gets its own file");
        assert!(a.parent().unwrap().is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(a.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700, "user-only (env holds secrets)");
        }
    }
}

#[test]
fn autoinstall_is_zsh_only() {
    assert!(autoinstall_env("/bin/bash").is_empty());
    assert!(autoinstall_env("/usr/bin/fish").is_empty());
}

#[test]
fn shell_is_zsh_matches_by_basename() {
    assert!(shell_is_zsh("/bin/zsh"));
    assert!(shell_is_zsh("/opt/homebrew/bin/zsh"));
    assert!(!shell_is_zsh("/bin/bash"));
}

#[test]
fn autoinstall_zsh_sets_zdotdir_to_a_real_dir() {
    let env = autoinstall_env("/bin/zsh");
    // On a machine where the temp dir is writable this wires ZDOTDIR up; if the
    // write failed it's empty. Either is valid — but when present the dir must exist
    // and carry both startup files with the hooks.
    if let Some((_, dir)) = env.iter().find(|(k, _)| k == "ZDOTDIR") {
        let dir = std::path::Path::new(dir);
        assert!(dir.join(".zshenv").is_file());
        let zshrc = std::fs::read_to_string(dir.join(".zshrc")).unwrap();
        assert!(zshrc.contains("133;C") && zshrc.contains("133;D"));
        assert!(
            zshrc.contains("source \"$ZDOTDIR/.zshrc\""),
            "sources user rc"
        );
    }
}
