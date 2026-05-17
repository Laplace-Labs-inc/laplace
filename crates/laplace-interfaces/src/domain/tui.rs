//! # TUI Capabilities: Data-Driven UI Permission Model
//!
//! This module defines the `TuiCapabilities` structure, which implements the "Dumb UI & Closed Kernel"
//! security model. The TUI receives a capabilities object from the kernel that explicitly defines
//! what panels, features, and data it is allowed to display. This ensures that tier-specific
//! information is withheld at the data source (kernel), not in UI code logic.
//!
//! ## Design Principle
//!
//! Rather than having the TUI contain conditional logic like:
//! ```ignore
//! if user_tier == Tier::Enterprise {
//!     // show sovereign panels
//! }
//! ```
//!
//! We instead inject a `TuiCapabilities` object that says:
//! ```ignore
//! let caps = TuiCapabilities {
//!     allowed_panels: vec![PanelType::Kraken, PanelType::Axiom, PanelType::Sovereign],
//!     enable_ttd: true,
//!     refresh_rate_ms: 100,
//!     tier: Tier::Enterprise,
//! };
//! ```
//!
//! The TUI renders based on what it's told to display, eliminating any possibility of circumvention
//! through code-level inspection. This is "physical data withholding" — the kernel simply doesn't
//! send the data in the first place.

use serde::{Deserialize, Serialize};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Represents the user's subscription tier, determining feature access and resource limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "UPPERCASE")]
pub enum Tier {
    /// Free tier: minimal monitoring, single-split layout, no TTD
    Free = 0,
    /// Professional tier: 4-split layout, basic TTD, API stats
    Pro = 1,
    /// Ultra tier: 4-split layout with enhanced metrics, full TTD, chaos stats
    Ultra = 2,
    /// Enterprise tier: 6-split god mode with Sovereign panels, full stealth capabilities
    Enterprise = 3,
}

impl Tier {
    /// Returns a human-readable name for the tier.
    pub fn display_name(&self) -> &'static str {
        match self {
            Tier::Free => "Free",
            Tier::Pro => "Pro",
            Tier::Ultra => "Ultra",
            Tier::Enterprise => "Enterprise",
        }
    }

    /// Returns the priority level (higher = more resources)
    pub fn priority(&self) -> u32 {
        match self {
            Tier::Free => 0,
            Tier::Pro => 1,
            Tier::Ultra => 2,
            Tier::Enterprise => 3,
        }
    }
}

/// Represents a specific panel type that can be rendered in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PanelType {
    /// Kraken load testing dashboard (core, available to all)
    Kraken,
    /// Axiom verification engine status (available to all)
    Axiom,
    /// API statistics and request metrics (Pro+)
    ApiStats,
    /// Chaos injection and event tracking (Ultra+)
    ChaosRadar,
    /// Resilience metrics and recovery patterns (Ultra+)
    ResilienceStats,
    /// Time-Travel Debugger causality replay (Pro+)
    TimeTravel,
    /// Sovereign capabilities and ownership verification (Enterprise only)
    Sovereign,
    /// Security events and authentication state (Enterprise only)
    SecurityAudit,
}

/// The canonical capabilities object injected from the kernel into the TUI.
///
/// This structure is immutable and serves as the single source of truth for what the TUI
/// is allowed to display. The kernel generates this based on the user's subscription tier,
/// license, and current authentication state.
///
/// **Example: Free tier TUI startup**
/// ```ignore
/// let caps = TuiCapabilities {
///     allowed_panels: vec![PanelType::Kraken, PanelType::Axiom],
///     enable_ttd: false,
///     refresh_rate_ms: 500,
///     tier: Tier::Free,
///     signature: Some("LAPLACE-SIG-2026-FREE-ABC123"),
///     is_authenticated: true,
/// };
/// ```
///
/// **Example: Enterprise tier with all capabilities**
/// ```ignore
/// let caps = TuiCapabilities {
///     allowed_panels: vec![
///         PanelType::Kraken, PanelType::Axiom, PanelType::ApiStats,
///         PanelType::ChaosRadar, PanelType::ResilienceStats,
///         PanelType::TimeTravel, PanelType::Sovereign, PanelType::SecurityAudit,
///     ],
///     enable_ttd: true,
///     refresh_rate_ms: 100,
///     tier: Tier::Enterprise,
///     signature: Some("LAPLACE-SIG-2026-ENT-XYZ789"),
///     is_authenticated: true,
/// };
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Tui",
        link = "LEP-0016-laplace-interfaces-dumb_ui_security"
    )
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiCapabilities {
    /// List of panels this TUI instance is allowed to render.
    /// Panels not in this list are simply skipped in layout calculations.
    pub allowed_panels: Vec<PanelType>,

    /// Whether Time-Travel Debugger features are enabled for this tier.
    pub enable_ttd: bool,

    /// Refresh interval in milliseconds. Free tier uses 500ms to reduce CPU load.
    /// Pro+ uses 100-200ms for real-time responsiveness.
    pub refresh_rate_ms: u64,

    /// The subscription tier that generated this capabilities object.
    pub tier: Tier,

    /// Laplace ownership signature (HMAC-SHA256 of tier + timestamp + API key).
    /// If None, TUI is in "unauthenticated" state.
    pub signature: Option<String>,

    /// Whether the current authentication is valid.
    /// If false, UI displays red-alert state and withholds all high-tier data.
    pub is_authenticated: bool,
}

impl TuiCapabilities {
    /// Creates a new TuiCapabilities for the Free tier (minimal access).
    pub fn free() -> Self {
        Self {
            allowed_panels: vec![PanelType::Kraken, PanelType::Axiom],
            enable_ttd: false,
            refresh_rate_ms: 500,
            tier: Tier::Free,
            signature: None,
            is_authenticated: false,
        }
    }

    /// Creates a new TuiCapabilities for the Pro tier (moderate access).
    pub fn pro() -> Self {
        Self {
            allowed_panels: vec![
                PanelType::Kraken,
                PanelType::Axiom,
                PanelType::ApiStats,
                PanelType::TimeTravel,
            ],
            enable_ttd: true,
            refresh_rate_ms: 200,
            tier: Tier::Pro,
            signature: Some("LAPLACE-SIG-2026-PRO-DEMO".to_string()),
            is_authenticated: true,
        }
    }

    /// Creates a new TuiCapabilities for the Ultra tier (comprehensive monitoring).
    pub fn ultra() -> Self {
        Self {
            allowed_panels: vec![
                PanelType::Kraken,
                PanelType::Axiom,
                PanelType::ApiStats,
                PanelType::ChaosRadar,
                PanelType::ResilienceStats,
                PanelType::TimeTravel,
            ],
            enable_ttd: true,
            refresh_rate_ms: 100,
            tier: Tier::Ultra,
            signature: Some("LAPLACE-SIG-2026-ULTRA-DEMO".to_string()),
            is_authenticated: true,
        }
    }

    /// Creates a new TuiCapabilities for the Enterprise tier (full access).
    pub fn enterprise() -> Self {
        Self {
            allowed_panels: vec![
                PanelType::Kraken,
                PanelType::Axiom,
                PanelType::ApiStats,
                PanelType::ChaosRadar,
                PanelType::ResilienceStats,
                PanelType::TimeTravel,
                PanelType::Sovereign,
                PanelType::SecurityAudit,
            ],
            enable_ttd: true,
            refresh_rate_ms: 100,
            tier: Tier::Enterprise,
            signature: Some("LAPLACE-SIG-2026-ENT-DEMO".to_string()),
            is_authenticated: true,
        }
    }

    /// Checks if a specific panel is allowed to be rendered.
    pub fn has_panel(&self, panel: PanelType) -> bool {
        self.allowed_panels.contains(&panel)
    }

    /// Checks if any of the given panels are allowed.
    pub fn has_any_panel(&self, panels: &[PanelType]) -> bool {
        panels.iter().any(|p| self.has_panel(*p))
    }

    /// Returns the authentication status as a display string.
    pub fn auth_display(&self) -> &'static str {
        if self.is_authenticated {
            "✓ Active"
        } else {
            "✗ Failed"
        }
    }

    /// Returns the signature display, showing ownership verification.
    pub fn signature_display(&self) -> String {
        match &self.signature {
            Some(sig) => format!("Laplace-Signature: {} ✓", sig),
            None => "Laplace-Signature: Not Provided".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_ordering() {
        assert!(Tier::Free < Tier::Pro);
        assert!(Tier::Pro < Tier::Ultra);
        assert!(Tier::Ultra < Tier::Enterprise);
    }

    #[test]
    fn tier_priorities() {
        assert_eq!(Tier::Free.priority(), 0);
        assert_eq!(Tier::Enterprise.priority(), 3);
    }

    #[test]
    fn free_tier_capabilities() {
        let caps = TuiCapabilities::free();
        assert_eq!(caps.tier, Tier::Free);
        assert!(!caps.enable_ttd);
        assert!(!caps.is_authenticated);
        assert_eq!(caps.allowed_panels.len(), 2);
        assert!(caps.has_panel(PanelType::Kraken));
        assert!(!caps.has_panel(PanelType::Sovereign));
    }

    #[test]
    fn enterprise_tier_capabilities() {
        let caps = TuiCapabilities::enterprise();
        assert_eq!(caps.tier, Tier::Enterprise);
        assert!(caps.enable_ttd);
        assert!(caps.is_authenticated);
        assert_eq!(caps.allowed_panels.len(), 8);
        assert!(caps.has_panel(PanelType::Sovereign));
        assert!(caps.has_panel(PanelType::SecurityAudit));
    }

    #[test]
    fn signature_display() {
        let free = TuiCapabilities::free();
        let enterprise = TuiCapabilities::enterprise();

        assert!(free.signature_display().contains("Not Provided"));
        assert!(enterprise
            .signature_display()
            .contains("LAPLACE-SIG-2026-ENT-DEMO"));
    }
}
