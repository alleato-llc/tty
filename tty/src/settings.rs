//! tty's persisted preferences — a small `tty.settings.json` in the user config dir.
//! The terminal counterpart of fed's `patina::Settings`, scoped to what a terminal
//! actually needs: dark/light, font, and an optional custom palette (the 16 ANSI
//! colors + fg/bg/cursor) edited in the settings panel or imported from a base16
//! scheme.

use std::path::PathBuf;

use iced::Color;
use rime::theme::{color_hex, parse_color};

use phosphor::TerminalStyle;

use crate::history::crypto::Cipher;

/// The lowest unfocused opacity we allow (5% → 95% transparency). A floor keeps the
/// window from fading to fully invisible and unrecoverable.
pub const MIN_OPACITY: f32 = 0.05;

/// The lowest *focused* opacity we allow (50% → 50% transparency). A higher floor
/// than [`MIN_OPACITY`]: a window you're actively using should stay clearly
/// readable, so active transparency tops out at 50%.
pub const MIN_FOCUSED_OPACITY: f32 = 0.5;

/// A custom terminal palette as hex strings (so it round-trips through JSON cleanly).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Palette {
    /// The 16 ANSI colors (0–7 normal, 8–15 bright).
    pub ansi: Vec<String>,
    pub fg: String,
    pub bg: String,
    pub cursor: String,
}

/// One `output_line_overrides` entry — e.g. `{ pattern: "tail *", max_lines: 200 }` to
/// let a `tail -f` capture more than the global default before it stops growing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutputLineOverride {
    pub pattern: String,
    pub max_lines: usize,
}

/// A metric that can appear in the status bar. CPU, memory, and the four
/// network/disk throughput rates have live samplers today (see `metrics.rs`);
/// swap/load arrive with their samplers later, so the set grows from here.
/// Network/disk are macOS-only for now (the Linux samplers are a follow-up).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    Cpu,
    /// CPU, drilling into the per-core grid only (same status-bar cell as `Cpu`).
    CpuCores,
    /// CPU, drilling into the aggregate line chart *and* the per-core grid (same
    /// status-bar cell as `Cpu`).
    CpuAll,
    Mem,
    NetRx,
    NetTx,
    DiskR,
    DiskW,
    /// Network rx + tx on a single sparkline (two overlaid series).
    NetIo,
    /// Disk read + write on a single sparkline (two overlaid series).
    DiskIo,
    /// System uptime (time since boot). A text cell, not a sparkline; drills into
    /// the full breakdown.
    Uptime,
    /// This terminal session's uptime (time since it launched). A text cell;
    /// drills into the full breakdown.
    Session,
    /// The current wall-clock time. A text cell (configurable format); drills
    /// into the full date. Refreshed by its own 1s timer, not the sampler.
    Clock,
    /// System load average. A sparkline of the 1-minute load; drills into the
    /// 1/5/15-minute triple.
    Load,
    /// Battery charge. A fixed 0..100% gauge sparkline; drills into the charging
    /// state and time estimate. Hidden on a machine with no battery.
    Battery,
}

impl MetricKind {
    /// Every metric that has a sampler today, in a stable order (used to offer
    /// the not-yet-added metrics in the settings editor).
    pub const ALL: [MetricKind; 15] = [
        MetricKind::Cpu,
        MetricKind::CpuCores,
        MetricKind::CpuAll,
        MetricKind::Mem,
        MetricKind::NetRx,
        MetricKind::NetTx,
        MetricKind::DiskR,
        MetricKind::DiskW,
        MetricKind::NetIo,
        MetricKind::DiskIo,
        MetricKind::Uptime,
        MetricKind::Session,
        MetricKind::Clock,
        MetricKind::Load,
        MetricKind::Battery,
    ];

    /// Whether this kind is a text uptime cell (system or session) rather than a
    /// sampled sparkline — it renders as text and drills into a full breakdown.
    pub fn is_uptime(self) -> bool {
        matches!(self, MetricKind::Uptime | MetricKind::Session)
    }

    /// Whether this kind is one of the CPU drill-ins (they share the aggregate
    /// status-bar cell and sampler; only their popover body differs).
    pub fn is_cpu(self) -> bool {
        matches!(
            self,
            MetricKind::Cpu | MetricKind::CpuCores | MetricKind::CpuAll
        )
    }

    /// The `status_bar_metrics[].metric` string this kind is stored as.
    pub fn as_setting_str(self) -> &'static str {
        match self {
            MetricKind::Cpu => "cpu",
            MetricKind::CpuCores => "cpu_cores",
            MetricKind::CpuAll => "cpu_all",
            MetricKind::Mem => "mem",
            MetricKind::NetRx => "net_rx",
            MetricKind::NetTx => "net_tx",
            MetricKind::DiskR => "disk_r",
            MetricKind::DiskW => "disk_w",
            MetricKind::NetIo => "net_io",
            MetricKind::DiskIo => "disk_io",
            MetricKind::Uptime => "uptime",
            MetricKind::Session => "session",
            MetricKind::Clock => "clock",
            MetricKind::Load => "load",
            MetricKind::Battery => "battery",
        }
    }

    /// Parse a `status_bar_metrics[].metric` value (unknown = not a metric we
    /// can render, so the caller drops it).
    pub fn from_setting_str(s: &str) -> Option<Self> {
        match s {
            "cpu" => Some(MetricKind::Cpu),
            "cpu_cores" => Some(MetricKind::CpuCores),
            "cpu_all" => Some(MetricKind::CpuAll),
            "mem" => Some(MetricKind::Mem),
            "net_rx" => Some(MetricKind::NetRx),
            "net_tx" => Some(MetricKind::NetTx),
            "disk_r" => Some(MetricKind::DiskR),
            "disk_w" => Some(MetricKind::DiskW),
            "net_io" => Some(MetricKind::NetIo),
            "disk_io" => Some(MetricKind::DiskIo),
            "uptime" => Some(MetricKind::Uptime),
            "session" => Some(MetricKind::Session),
            "clock" => Some(MetricKind::Clock),
            "load" => Some(MetricKind::Load),
            "battery" => Some(MetricKind::Battery),
            _ => None,
        }
    }
}

impl std::fmt::Display for MetricKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MetricKind::Cpu => "CPU",
            MetricKind::CpuCores => "CPU Cores",
            MetricKind::CpuAll => "CPU (all)",
            MetricKind::Mem => "Memory",
            MetricKind::NetRx => "Net RX",
            MetricKind::NetTx => "Net TX",
            MetricKind::DiskR => "Disk R",
            MetricKind::DiskW => "Disk W",
            MetricKind::NetIo => "Net I/O",
            MetricKind::DiskIo => "Disk I/O",
            MetricKind::Uptime => "Uptime",
            MetricKind::Session => "Session",
            MetricKind::Clock => "Clock",
            MetricKind::Load => "Load",
            MetricKind::Battery => "Battery",
        })
    }
}

/// How a metric renders in the status bar. `Sparkline` is the filled mini
/// line-chart; `Number` is the plain label only (the Phase 1 look). Gauge/rate
/// styles from the design sketch come with the metrics that need them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricStyle {
    #[default]
    Sparkline,
    Number,
}

impl MetricStyle {
    /// Every style, for the settings editor's per-metric dropdown.
    pub const ALL: [MetricStyle; 2] = [MetricStyle::Sparkline, MetricStyle::Number];

    /// The `status_bar_metrics[].style` string this style is stored as.
    pub fn as_setting_str(self) -> &'static str {
        match self {
            MetricStyle::Sparkline => "sparkline",
            MetricStyle::Number => "number",
        }
    }

    /// Parse a `status_bar_metrics[].style` value (absent/unknown = the default
    /// sparkline).
    pub fn from_setting_str(s: &str) -> Self {
        match s {
            "number" => MetricStyle::Number,
            _ => MetricStyle::Sparkline,
        }
    }
}

impl std::fmt::Display for MetricStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MetricStyle::Sparkline => "Sparkline",
            MetricStyle::Number => "Number",
        })
    }
}

/// One entry in the ordered [`Settings::status_bar_metrics`] list: a metric and
/// how it renders. The list's order is the bar's left-to-right display order.
/// Stored as strings (like `history_cipher` and friends) so an unrecognized
/// `metric` from a hand-edit or a newer tty parses leniently — dropped by
/// [`Settings::status_bar_metrics`] rather than failing the whole settings load.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MetricConfig {
    pub metric: String,
    #[serde(default)]
    pub style: String,
}

/// A [`MetricConfig`] resolved to its typed metric and style — the form the
/// status bar and its editor render. Unknown entries are dropped in the resolve
/// (see [`Settings::status_bar_metrics`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedMetric {
    pub kind: MetricKind,
    pub style: MetricStyle,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// The built-in theme name (e.g. `"Dracula"`, `"Nord"`); absent reads as Dracula.
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub font_size: Option<f32>,
    /// A custom palette overriding the built-in dark/light terminal colors.
    #[serde(default)]
    pub palette: Option<Palette>,
    /// Terminal-background opacity when the window is **unfocused** (`1.0` = opaque =
    /// the feature off; lower = more see-through). Down to [`MIN_OPACITY`].
    #[serde(default)]
    pub unfocused_opacity: Option<f32>,
    /// Terminal-background opacity when the window **is focused** (`1.0`/absent =
    /// opaque = the feature off; lower = more see-through). Floored at
    /// [`MIN_FOCUSED_OPACITY`] so an in-use window stays readable.
    #[serde(default)]
    pub focused_opacity: Option<f32>,
    /// Keep the window above other windows (`true`), or let it order normally
    /// (`false`/absent, the default).
    #[serde(default)]
    pub window_always_on_top: Option<bool>,
    /// Clock cell: 24-hour time (`true`) vs 12-hour with AM/PM (`false`/absent).
    #[serde(default)]
    pub clock_24h: Option<bool>,
    /// Clock cell: show seconds (`true`) vs minute precision (`false`/absent).
    #[serde(default)]
    pub clock_seconds: Option<bool>,
    /// Clock cell: prefix the weekday + date (`true`) vs time only
    /// (`false`/absent).
    #[serde(default)]
    pub clock_date: Option<bool>,
    /// Ink the active tab with the accent color (`true`/absent) or with a subtler
    /// normal-ink emphasis (`false`). Either way the active tab reads as active versus
    /// the muted inactive tabs; this just dials the loudness.
    #[serde(default)]
    pub tab_highlight: Option<bool>,
    /// Hide the bottom status bar until the pointer nears the bottom edge
    /// (`true`/absent, the default), or keep it always visible (`false`). When
    /// on, the bar floats over the bottom edge on near-hover instead of taking
    /// a row, so revealing it never reflows the terminal grid. Ignored when
    /// [`Self::status_bar_disabled`] is on (the bar never shows at all).
    #[serde(default)]
    pub status_bar_autohide: Option<bool>,
    /// Turn the status bar off entirely (`true`): it never shows, not even on
    /// near-hover, and gives the terminal the full height. Absent/`false` (the
    /// default) keeps it, governed by [`Self::status_bar_autohide`].
    #[serde(default)]
    pub status_bar_disabled: Option<bool>,
    /// The ordered list of live machine-stat cells shown in the status bar —
    /// each an entry `{ metric, style }`. The list order is the display order;
    /// an empty (absent) list means the bar shows no stats, the default. When
    /// non-empty, `prexp-core` is sampled every
    /// [`Self::status_bar_metrics_interval_ms`]; see `metrics.rs`. Only metrics
    /// with a sampler ([`MetricKind::ALL`]) are kept on load.
    #[serde(default)]
    pub status_bar_metrics: Vec<MetricConfig>,
    /// How often (milliseconds) to resample machine stats while at least one
    /// metric is shown. Absent = [`DEFAULT_METRICS_INTERVAL_MS`]; clamped to a
    /// sane range so a bad value can't peg the sampler or stall it.
    #[serde(default)]
    pub status_bar_metrics_interval_ms: Option<u64>,
    /// Keep metric drill-in popovers open on a click away (`true`), so several
    /// can stay pinned side by side, each dismissed by its own close button or
    /// Escape-closes-all. Absent/`false` (the default) is the one-at-a-time mode
    /// where clicking a metric replaces any open popover and a click away closes
    /// it.
    #[serde(default)]
    pub status_bar_metrics_pinned: Option<bool>,
    /// Deprecated: the old on/off machine-stats toggle, replaced by the ordered
    /// [`Self::status_bar_metrics`] list. Read only to migrate an existing
    /// `true` into `[cpu, mem]` on load (see [`Self::load`]); never written back.
    #[serde(default, skip_serializing)]
    pub status_bar_metrics_enabled: Option<bool>,
    /// How many scrollback lines each terminal keeps before evicting the oldest.
    #[serde(default)]
    pub max_scrollback: Option<usize>,
    /// How many output lines a command keeps by default (a long-running/streaming
    /// command like `tail -f` just stops growing past this).
    #[serde(default)]
    pub default_output_lines: Option<usize>,
    /// Per-command overrides: the first glob pattern (checked in order) matching the
    /// command's text wins over `default_output_lines`.
    #[serde(default)]
    pub output_line_overrides: Vec<OutputLineOverride>,
    /// Whether the Scrollback History panel's command log persists across
    /// launches, encrypted at rest. Off (absent) by default — opt-in, since it
    /// changes what's written to disk. Toggling this on triggers first-time
    /// keychain key creation; toggling it off does not delete an existing
    /// archive (see the settings panel for that, a separate, explicit action).
    #[serde(default)]
    pub encrypted_history_enabled: Option<bool>,
    /// Where the encryption key comes from: `"keychain"` (default) or
    /// `"passphrase"` (a KDF over a user passphrase — see
    /// `history::passphrase`). Like the cipher, fixed once the archive has
    /// data: changing it requires a Reset.
    #[serde(default)]
    pub history_key_source: Option<String>,
    /// Which KDF stretches the passphrase (passphrase key source only):
    /// `"argon2id"` (default), `"scrypt"`, or `"pbkdf2"`. Chooses the recipe
    /// for *new* archives; an existing archive always uses the recipe
    /// recorded in its own KDF sidecar, whatever this says now.
    #[serde(default)]
    pub history_kdf: Option<String>,
    /// Which cipher encrypts the history archive: `"chacha20poly1305"`
    /// (default) or `"dorado"`. Fixed once the archive has any data in it —
    /// this is a first-enable choice, not something the settings panel lets
    /// you change after the fact without starting over. See
    /// [`crate::history::crypto::Cipher`].
    #[serde(default)]
    pub history_cipher: Option<String>,
    /// Which PRF fans the master key out into the per-purpose child keys
    /// (`history::HistoryKeys`): `"auto"` (default — match the cipher's
    /// family: BLAKE3 for ChaCha20-Poly1305, Skein-512 for Threefish),
    /// `"skein512"`, or `"blake3"`. Like the cipher and key source, fixed once
    /// the archive has data: changing it means a Reset. See
    /// [`HistoryFanout`].
    #[serde(default)]
    pub history_fanout: Option<String>,
    /// How often (minutes) to require a fresh Touch ID/device-password check
    /// before opening the Scrollback History panel, on top of the always-on
    /// once-per-session gate. `0`/absent = only the once-per-session gate.
    /// macOS only — see `history::reauth`.
    #[serde(default)]
    pub history_reauth_interval_minutes: Option<u32>,
    /// What a launch does about recording, when encrypted history is on:
    /// `"record"` (default) starts recording; `"ask"` asks each launch;
    /// `"untracked"` starts the whole session untracked (nothing recorded,
    /// no key read) until the next launch. The `--untracked` CLI flag
    /// overrides all three for one launch.
    #[serde(default)]
    pub history_session_start: Option<String>,
}

/// [`Settings::max_scrollback`]'s default and clamp range — matches `cathode`'s
/// historical hardcoded default, now user-adjustable between these bounds.
pub const DEFAULT_MAX_SCROLLBACK: usize = 2000;
pub const MIN_MAX_SCROLLBACK: usize = 200;
pub const MAX_MAX_SCROLLBACK: usize = 20_000;

/// [`Settings::default_output_lines`]'s default and clamp range.
pub const DEFAULT_OUTPUT_LINES: usize = 50;
pub const MIN_OUTPUT_LINES: usize = 1;
pub const MAX_OUTPUT_LINES: usize = 10_000;

/// [`Settings::status_bar_metrics_interval_ms`]'s default and clamp range. The
/// design sketch's 2s cadence is the default; the floor keeps a rogue value
/// from resampling every frame, the ceiling from stalling to a near-freeze.
pub const DEFAULT_METRICS_INTERVAL_MS: u64 = 2000;
pub const MIN_METRICS_INTERVAL_MS: u64 = 250;
pub const MAX_METRICS_INTERVAL_MS: u64 = 60_000;

/// Where the encrypted-history key comes from — the resolved form of
/// [`Settings::history_key_source`], mirroring how `Cipher` pairs with
/// `Settings::history_cipher`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// A random 256-bit key stored in the OS keychain (the default).
    Keychain,
    /// A key derived from a user passphrase with Argon2id — for platforms
    /// or runtimes with no usable keychain, or by preference.
    Passphrase,
}

impl KeySource {
    /// The `Settings::history_key_source` string this source is stored as.
    pub fn as_setting_str(self) -> &'static str {
        match self {
            KeySource::Keychain => "keychain",
            KeySource::Passphrase => "passphrase",
        }
    }

    /// Parse a `Settings::history_key_source` value (absent or unrecognized
    /// falls back to the keychain).
    pub fn from_setting_str(s: Option<&str>) -> Self {
        match s {
            Some("passphrase") => KeySource::Passphrase,
            _ => KeySource::Keychain,
        }
    }
}

impl std::fmt::Display for KeySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            KeySource::Keychain => "OS Keychain",
            KeySource::Passphrase => "Passphrase",
        })
    }
}

/// Which KDF stretches a history passphrase into the archive key — the
/// resolved form of [`Settings::history_kdf`], mirroring how `Cipher` pairs
/// with `Settings::history_cipher`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKdf {
    /// Memory-hard, the Password Hashing Competition winner and current
    /// best-practice default.
    Argon2id,
    /// Memory-hard, older; a fine alternative.
    Scrypt,
    /// Compute-hard only (not memory-hard) — kept for environments that
    /// standardize on it.
    Pbkdf2,
}

impl HistoryKdf {
    /// The `Settings::history_kdf` string this KDF is stored/selected as.
    pub fn as_setting_str(self) -> &'static str {
        match self {
            HistoryKdf::Argon2id => "argon2id",
            HistoryKdf::Scrypt => "scrypt",
            HistoryKdf::Pbkdf2 => "pbkdf2",
        }
    }

    /// Parse a `Settings::history_kdf` value (absent or unrecognized falls
    /// back to Argon2id — the alternatives are deliberate opt-ins).
    pub fn from_setting_str(s: Option<&str>) -> Self {
        match s {
            Some("scrypt") => HistoryKdf::Scrypt,
            Some("pbkdf2") => HistoryKdf::Pbkdf2,
            _ => HistoryKdf::Argon2id,
        }
    }
}

impl std::fmt::Display for HistoryKdf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HistoryKdf::Argon2id => "Argon2id",
            HistoryKdf::Scrypt => "scrypt",
            HistoryKdf::Pbkdf2 => "PBKDF2-SHA256",
        })
    }
}

/// Which PRF fans the history master key out into its per-purpose children —
/// the resolved form of [`Settings::history_fanout`]. Both Skein-512 and
/// BLAKE3 are secure PRFs and produce equally strong keys; the choice only
/// keeps a construction within one cryptographic family. `Auto` follows the
/// cipher (see [`Self::resolve`]); the two concrete variants are an explicit
/// override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryFanout {
    /// Match the cipher's family: BLAKE3 for ChaCha20-Poly1305, Skein-512 for
    /// the Threefish (dorado) cipher. The default.
    Auto,
    /// Force the Skein-512 keyed hash regardless of cipher.
    Skein512,
    /// Force the BLAKE3 keyed hash regardless of cipher.
    Blake3,
}

impl HistoryFanout {
    /// The `Settings::history_fanout` string this choice is stored/selected as.
    pub fn as_setting_str(self) -> &'static str {
        match self {
            HistoryFanout::Auto => "auto",
            HistoryFanout::Skein512 => "skein512",
            HistoryFanout::Blake3 => "blake3",
        }
    }

    /// Parse a `Settings::history_fanout` value (absent or unrecognized falls
    /// back to `Auto` — matching the cipher's family is the sensible default).
    pub fn from_setting_str(s: Option<&str>) -> Self {
        match s {
            Some("skein512") => HistoryFanout::Skein512,
            Some("blake3") => HistoryFanout::Blake3,
            _ => HistoryFanout::Auto,
        }
    }

    /// The concrete `dorado_engine` PRF this choice resolves to for `cipher`.
    /// `Auto` matches the cipher's family; the overrides ignore `cipher`.
    pub fn resolve(self, cipher: Cipher) -> dorado_engine::kdf::KdfPrf {
        use dorado_engine::kdf::KdfPrf;
        match self {
            HistoryFanout::Skein512 => KdfPrf::Skein512,
            HistoryFanout::Blake3 => KdfPrf::Blake3,
            HistoryFanout::Auto => match cipher {
                Cipher::ChaCha20Poly1305 => KdfPrf::Blake3,
                Cipher::DoradoRawAuthenticated => KdfPrf::Skein512,
            },
        }
    }

    /// The PRF `Auto` resolves to for `cipher`, for labeling the `Auto` option
    /// in the UI (e.g. "Auto (BLAKE3)").
    pub fn auto_label(cipher: Cipher) -> &'static str {
        match cipher {
            Cipher::ChaCha20Poly1305 => "BLAKE3",
            Cipher::DoradoRawAuthenticated => "Skein-512",
        }
    }
}

impl std::fmt::Display for HistoryFanout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HistoryFanout::Auto => "Auto",
            HistoryFanout::Skein512 => "Skein-512",
            HistoryFanout::Blake3 => "BLAKE3",
        })
    }
}

/// What a launch does about recording — the resolved form of
/// [`Settings::history_session_start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStart {
    /// Start recording right away (the default).
    Record,
    /// Ask each launch: record, or stay untracked?
    Ask,
    /// Start the whole session untracked — nothing recorded, no key read.
    Untracked,
}

impl SessionStart {
    /// The `Settings::history_session_start` string this mode is stored as.
    pub fn as_setting_str(self) -> &'static str {
        match self {
            SessionStart::Record => "record",
            SessionStart::Ask => "ask",
            SessionStart::Untracked => "untracked",
        }
    }

    /// Parse a `Settings::history_session_start` value (absent or
    /// unrecognized falls back to `Record`, the long-standing behavior).
    pub fn from_setting_str(s: Option<&str>) -> Self {
        match s {
            Some("ask") => SessionStart::Ask,
            Some("untracked") => SessionStart::Untracked,
            _ => SessionStart::Record,
        }
    }
}

impl std::fmt::Display for SessionStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SessionStart::Record => "Record",
            SessionStart::Ask => "Ask each launch",
            SessionStart::Untracked => "Start untracked",
        })
    }
}

/// [`Settings::history_reauth_interval_minutes`]'s clamp range. `0` means
/// "off" (only the once-per-session gate applies), not "immediately".
pub const MIN_HISTORY_REAUTH_INTERVAL_MINUTES: u32 = 0;
pub const MAX_HISTORY_REAUTH_INTERVAL_MINUTES: u32 = 480;

impl Settings {
    /// Load `tty.settings.json`, or defaults if it's missing or malformed.
    pub fn load() -> Self {
        let mut settings: Self = match std::fs::read_to_string(path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        settings.migrate_status_bar_metrics();
        settings
    }

    /// Migrate the deprecated `status_bar_metrics_enabled` bool: if the old
    /// toggle was on and no ordered list has been configured yet, seed the list
    /// with CPU + memory (the pair the old toggle showed) so an upgrade keeps
    /// stats visible. The deprecated flag is then cleared and never re-written.
    fn migrate_status_bar_metrics(&mut self) {
        if self.status_bar_metrics_enabled == Some(true) && self.status_bar_metrics.is_empty() {
            self.status_bar_metrics = vec![
                MetricConfig {
                    metric: MetricKind::Cpu.as_setting_str().to_string(),
                    style: MetricStyle::Sparkline.as_setting_str().to_string(),
                },
                MetricConfig {
                    metric: MetricKind::Mem.as_setting_str().to_string(),
                    style: MetricStyle::Sparkline.as_setting_str().to_string(),
                },
            ];
        }
        self.status_bar_metrics_enabled = None;
    }

    /// Persist to `tty.settings.json` (best-effort; a write failure isn't fatal).
    pub fn save(&self) {
        // `path()` is the real config file — behavior tests drive `update()`
        // paths that save, and a test run must never rewrite the settings of
        // whoever ran `cargo test`.
        if cfg!(test) {
            return;
        }
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, json);
        }
    }

    /// The unfocused-window opacity (`1.0` = opaque/off), clamped so the window can
    /// never become fully invisible (down to 5% opacity / 95% transparency).
    pub fn unfocused_opacity(&self) -> f32 {
        self.unfocused_opacity
            .unwrap_or(1.0)
            .clamp(MIN_OPACITY, 1.0)
    }

    /// The focused-window opacity (`1.0` = opaque/off), clamped to at least
    /// [`MIN_FOCUSED_OPACITY`] (50%) so an in-use window stays readable.
    pub fn focused_opacity(&self) -> f32 {
        self.focused_opacity
            .unwrap_or(1.0)
            .clamp(MIN_FOCUSED_OPACITY, 1.0)
    }

    /// Whether the window should stay above other windows (default `false`).
    pub fn window_always_on_top(&self) -> bool {
        self.window_always_on_top.unwrap_or(false)
    }

    /// The clock cell's format, resolved from the individual toggles.
    pub fn clock_format(&self) -> crate::metrics::ClockFormat {
        crate::metrics::ClockFormat {
            hour24: self.clock_24h.unwrap_or(false),
            seconds: self.clock_seconds.unwrap_or(false),
            date: self.clock_date.unwrap_or(false),
        }
    }

    /// Whether to ink the active tab with the accent (default `true`).
    pub fn tab_highlight(&self) -> bool {
        self.tab_highlight.unwrap_or(true)
    }

    /// Whether the status bar auto-hides until the pointer nears the bottom
    /// edge (default `true`).
    pub fn status_bar_autohide(&self) -> bool {
        self.status_bar_autohide.unwrap_or(true)
    }

    /// Whether the status bar is turned off entirely (default `false`).
    pub fn status_bar_disabled(&self) -> bool {
        self.status_bar_disabled.unwrap_or(false)
    }

    /// Whether metric popovers stay open on a click away so several can be
    /// pinned at once (default `false`, the one-at-a-time mode).
    pub fn status_bar_metrics_pinned(&self) -> bool {
        self.status_bar_metrics_pinned.unwrap_or(false)
    }

    /// The configured status-bar metrics resolved to typed metric + style, in
    /// display order, keeping only those with a live sampler (an unknown
    /// `metric` from a hand-edited or forward-version file is dropped rather
    /// than rendered blank).
    pub fn status_bar_metrics(&self) -> Vec<ResolvedMetric> {
        self.status_bar_metrics
            .iter()
            .filter_map(|c| {
                MetricKind::from_setting_str(&c.metric).map(|kind| ResolvedMetric {
                    kind,
                    style: MetricStyle::from_setting_str(&c.style),
                })
            })
            .collect()
    }

    /// The metrics resample interval (default [`DEFAULT_METRICS_INTERVAL_MS`]),
    /// clamped so it can neither peg the sampler nor stall it.
    pub fn status_bar_metrics_interval_ms(&self) -> u64 {
        self.status_bar_metrics_interval_ms
            .unwrap_or(DEFAULT_METRICS_INTERVAL_MS)
            .clamp(MIN_METRICS_INTERVAL_MS, MAX_METRICS_INTERVAL_MS)
    }

    /// Append `metric` (default sparkline) to the status-bar list, ignoring an
    /// unknown kind or one already present. Returns whether the list changed.
    pub fn add_status_bar_metric(&mut self, metric: &str) -> bool {
        let Some(kind) = MetricKind::from_setting_str(metric) else {
            return false;
        };
        let present = self
            .status_bar_metrics
            .iter()
            .any(|c| MetricKind::from_setting_str(&c.metric) == Some(kind));
        if present {
            return false;
        }
        self.status_bar_metrics.push(MetricConfig {
            metric: kind.as_setting_str().to_string(),
            style: MetricStyle::default().as_setting_str().to_string(),
        });
        true
    }

    /// Remove the status-bar metric at `idx` (a no-op if out of range).
    pub fn remove_status_bar_metric(&mut self, idx: usize) {
        if idx < self.status_bar_metrics.len() {
            self.status_bar_metrics.remove(idx);
        }
    }

    /// Move the status-bar metric at `idx` by `delta` (negative = toward the
    /// front/left), clamped to the ends. A no-op if `idx` is out of range or the
    /// move would leave the list unchanged.
    pub fn move_status_bar_metric(&mut self, idx: usize, delta: i32) {
        let len = self.status_bar_metrics.len();
        if idx >= len {
            return;
        }
        let target = (idx as i32 + delta).clamp(0, len as i32 - 1) as usize;
        if target != idx {
            let item = self.status_bar_metrics.remove(idx);
            self.status_bar_metrics.insert(target, item);
        }
    }

    /// Set the render style of the status-bar metric at `idx` (a no-op if out
    /// of range). Stored canonicalized so an odd input can't linger in the file.
    pub fn set_status_bar_metric_style(&mut self, idx: usize, style: &str) {
        if let Some(c) = self.status_bar_metrics.get_mut(idx) {
            c.style = MetricStyle::from_setting_str(style)
                .as_setting_str()
                .to_string();
        }
    }

    /// How many scrollback lines each terminal keeps (default 2000).
    pub fn max_scrollback(&self) -> usize {
        self.max_scrollback
            .unwrap_or(DEFAULT_MAX_SCROLLBACK)
            .clamp(MIN_MAX_SCROLLBACK, MAX_MAX_SCROLLBACK)
    }

    /// How many output lines a command keeps by default (default 50).
    pub fn default_output_lines(&self) -> usize {
        self.default_output_lines
            .unwrap_or(DEFAULT_OUTPUT_LINES)
            .clamp(MIN_OUTPUT_LINES, MAX_OUTPUT_LINES)
    }

    /// The configured overrides as `(pattern, cap)` pairs, ready for
    /// `cathode::commands::resolve_output_cap`.
    pub fn output_line_overrides(&self) -> Vec<(String, usize)> {
        self.output_line_overrides
            .iter()
            .map(|o| (o.pattern.clone(), o.max_lines))
            .collect()
    }

    /// The output-line cap for `command` — the first matching override, else
    /// [`Self::default_output_lines`].
    pub fn resolve_output_cap(&self, command: &str) -> usize {
        cathode::commands::resolve_output_cap(
            command,
            &self.output_line_overrides(),
            self.default_output_lines(),
        )
    }

    /// Whether encrypted history persistence is turned on (default off).
    pub fn encrypted_history_enabled(&self) -> bool {
        self.encrypted_history_enabled.unwrap_or(false)
    }

    /// The configured key source (falls back to the keychain for an absent
    /// or unrecognized value, mirroring [`Cipher::from_setting_str`] — the
    /// passphrase mode is a deliberate opt-in, not something a malformed
    /// settings file can select).
    pub fn history_key_source(&self) -> KeySource {
        KeySource::from_setting_str(self.history_key_source.as_deref())
    }

    /// The configured cipher, resolved to the real [`Cipher`] enum (falls
    /// back to `ChaCha20Poly1305` for an absent or unrecognized value — see
    /// [`Cipher::from_setting_str`]).
    pub fn history_cipher(&self) -> Cipher {
        Cipher::from_setting_str(self.history_cipher.as_deref())
    }

    /// The configured launch behavior (falls back to `Record` for an absent
    /// or unrecognized value).
    pub fn history_session_start(&self) -> SessionStart {
        SessionStart::from_setting_str(self.history_session_start.as_deref())
    }

    /// The configured passphrase KDF for *new* archives (falls back to
    /// Argon2id) — an existing archive uses its sidecar's recorded recipe.
    pub fn history_kdf(&self) -> HistoryKdf {
        HistoryKdf::from_setting_str(self.history_kdf.as_deref())
    }

    /// The configured fan-out PRF (falls back to `Auto`, which matches the
    /// cipher's family). Fixed at enable like the cipher; an existing archive
    /// must decrypt under the same choice or a Reset is required.
    pub fn history_fanout(&self) -> HistoryFanout {
        HistoryFanout::from_setting_str(self.history_fanout.as_deref())
    }

    /// The re-auth idle interval in minutes (default/clamp: `0`, meaning off
    /// — only the once-per-session gate applies).
    pub fn history_reauth_interval_minutes(&self) -> u32 {
        self.history_reauth_interval_minutes
            .unwrap_or(MIN_HISTORY_REAUTH_INTERVAL_MINUTES)
            .clamp(
                MIN_HISTORY_REAUTH_INTERVAL_MINUTES,
                MAX_HISTORY_REAUTH_INTERVAL_MINUTES,
            )
    }

    /// The custom [`TerminalStyle`] this file describes, if any (the panel/base16 edits).
    pub fn custom_style(&self) -> Option<TerminalStyle> {
        let p = self.palette.as_ref()?;
        if p.ansi.len() != 16 {
            return None;
        }
        let mut ansi = [Color::BLACK; 16];
        for (i, hex) in p.ansi.iter().enumerate() {
            ansi[i] = parse_color(hex)?;
        }
        Some(TerminalStyle {
            ansi,
            fg: parse_color(&p.fg)?,
            bg: parse_color(&p.bg)?,
            cursor: parse_color(&p.cursor)?,
            // Keep a readable selection tint derived from the chosen blue.
            selection: Color { a: 0.4, ..ansi[4] },
        })
    }

    /// Replace the custom palette from a fully-resolved [`TerminalStyle`] (panel edits +
    /// base16 import both funnel through here).
    pub fn set_palette(&mut self, style: &TerminalStyle) {
        self.palette = Some(Palette {
            ansi: style.ansi.iter().map(|c| color_hex(*c)).collect(),
            fg: color_hex(style.fg),
            bg: color_hex(style.bg),
            cursor: color_hex(style.cursor),
        });
    }
}

/// `~/.config/tty/tty.settings.json` (or the platform equivalent).
fn path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tty")
        .join("tty.settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resolved kinds of the configured metrics, in order.
    fn kinds(s: &Settings) -> Vec<MetricKind> {
        s.status_bar_metrics().iter().map(|c| c.kind).collect()
    }

    #[test]
    fn add_appends_dedupes_and_rejects_unknown() {
        let mut s = Settings::default();
        assert!(s.add_status_bar_metric("cpu"));
        assert!(s.add_status_bar_metric("mem"));
        // Duplicate and unknown are both refused, list unchanged.
        assert!(!s.add_status_bar_metric("cpu"));
        assert!(!s.add_status_bar_metric("gpu")); // not a metric we sample
        assert_eq!(kinds(&s), vec![MetricKind::Cpu, MetricKind::Mem]);
    }

    #[test]
    fn remove_drops_by_index_and_ignores_out_of_range() {
        let mut s = Settings::default();
        s.add_status_bar_metric("cpu");
        s.add_status_bar_metric("mem");
        s.remove_status_bar_metric(5); // no-op
        s.remove_status_bar_metric(0);
        assert_eq!(kinds(&s), vec![MetricKind::Mem]);
    }

    #[test]
    fn move_reorders_and_clamps_to_the_ends() {
        let mut s = Settings::default();
        s.add_status_bar_metric("cpu");
        s.add_status_bar_metric("mem");
        // Moving CPU down past the end lands it last (clamped), not out of range.
        s.move_status_bar_metric(0, 5);
        assert_eq!(kinds(&s), vec![MetricKind::Mem, MetricKind::Cpu]);
        // And back up past the front.
        s.move_status_bar_metric(1, -5);
        assert_eq!(kinds(&s), vec![MetricKind::Cpu, MetricKind::Mem]);
    }

    #[test]
    fn style_is_set_and_canonicalized() {
        let mut s = Settings::default();
        s.add_status_bar_metric("cpu");
        s.set_status_bar_metric_style(0, "number");
        assert_eq!(s.status_bar_metrics()[0].style, MetricStyle::Number);
        // An unrecognized style falls back to the default sparkline.
        s.set_status_bar_metric_style(0, "bogus");
        assert_eq!(s.status_bar_metrics()[0].style, MetricStyle::Sparkline);
        assert_eq!(
            s.status_bar_metrics.first().map(|c| c.style.as_str()),
            Some("sparkline")
        );
    }

    #[test]
    fn unknown_metric_is_dropped_from_resolved_not_fatal() {
        // A forward-version / hand-edited entry parses fine (strings) and is
        // simply skipped in the resolved list — it must not nuke the settings.
        let json = r#"{ "status_bar_metrics": [
            { "metric": "cpu", "style": "sparkline" },
            { "metric": "gpu", "style": "sparkline" },
            { "metric": "mem" }
        ] }"#;
        let s: Settings = serde_json::from_str(json).expect("lenient parse");
        assert_eq!(kinds(&s), vec![MetricKind::Cpu, MetricKind::Mem]);
        // Missing style defaults to sparkline.
        assert_eq!(s.status_bar_metrics()[1].style, MetricStyle::Sparkline);
    }

    #[test]
    fn deprecated_enabled_toggle_migrates_to_cpu_and_mem() {
        // An old settings file with the on/off toggle set, no ordered list yet.
        let mut s: Settings =
            serde_json::from_str(r#"{ "status_bar_metrics_enabled": true }"#).expect("parse");
        s.migrate_status_bar_metrics();
        assert_eq!(kinds(&s), vec![MetricKind::Cpu, MetricKind::Mem]);
        // The deprecated flag is cleared and never serialized back.
        assert_eq!(s.status_bar_metrics_enabled, None);
        assert!(!serde_json::to_string(&s)
            .unwrap()
            .contains("status_bar_metrics_enabled"));
    }

    #[test]
    fn migration_does_not_clobber_an_existing_list() {
        // A user who already configured the new list keeps it, toggle ignored.
        let mut s: Settings = serde_json::from_str(
            r#"{ "status_bar_metrics_enabled": true,
                 "status_bar_metrics": [{ "metric": "net_rx" }] }"#,
        )
        .expect("parse");
        s.migrate_status_bar_metrics();
        assert_eq!(kinds(&s), vec![MetricKind::NetRx]);
    }

    #[test]
    fn interval_defaults_and_clamps() {
        let mut s = Settings::default();
        assert_eq!(
            s.status_bar_metrics_interval_ms(),
            DEFAULT_METRICS_INTERVAL_MS
        );
        s.status_bar_metrics_interval_ms = Some(0);
        assert_eq!(s.status_bar_metrics_interval_ms(), MIN_METRICS_INTERVAL_MS);
        s.status_bar_metrics_interval_ms = Some(u64::MAX);
        assert_eq!(s.status_bar_metrics_interval_ms(), MAX_METRICS_INTERVAL_MS);
    }
}
