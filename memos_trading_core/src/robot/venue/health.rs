//! Venue sağlık sinyallerini tek bir execution-readiness kararına indirger.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VenueReadiness {
    Ready,
    Degraded,
    Blocked,
    Unknown,
    SignalOnly,
}

impl VenueReadiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "READY",
            Self::Degraded => "DEGRADED",
            Self::Blocked => "BLOCKED",
            Self::Unknown => "UNKNOWN",
            Self::SignalOnly => "SIGNAL_ONLY",
        }
    }
}

pub fn decide_readiness(
    signal_only: bool,
    has_blockers: bool,
    has_critical_blocker: bool,
    has_observed_state: bool,
) -> VenueReadiness {
    if signal_only {
        return VenueReadiness::SignalOnly;
    }
    if !has_observed_state {
        return VenueReadiness::Unknown;
    }
    if has_critical_blocker {
        return VenueReadiness::Blocked;
    }
    if has_blockers {
        return VenueReadiness::Degraded;
    }
    VenueReadiness::Ready
}

#[cfg(test)]
mod tests {
    use super::{decide_readiness, VenueReadiness};

    #[test]
    fn readiness_is_fail_closed_for_missing_state() {
        assert_eq!(
            decide_readiness(false, false, false, false),
            VenueReadiness::Unknown
        );
    }

    #[test]
    fn critical_blockers_win_over_degraded_state() {
        assert_eq!(
            decide_readiness(false, true, true, true),
            VenueReadiness::Blocked
        );
    }

    #[test]
    fn healthy_and_signal_only_states_are_distinct() {
        assert_eq!(
            decide_readiness(false, false, false, true).as_str(),
            "READY"
        );
        assert_eq!(
            decide_readiness(true, false, false, true).as_str(),
            "SIGNAL_ONLY"
        );
    }

    #[test]
    fn readiness_matrix_fails_closed_for_operational_blockers() {
        let cases = [
            (true, false, true, "DEGRADED"),
            (true, true, true, "BLOCKED"),
            (false, false, false, "UNKNOWN"),
        ];
        for (has_blockers, has_critical, observed, expected) in cases {
            assert_eq!(
                decide_readiness(false, has_blockers, has_critical, observed).as_str(),
                expected
            );
        }
    }

    #[test]
    fn signal_only_remains_safe_when_account_and_reconciliation_are_unavailable() {
        assert_eq!(
            decide_readiness(true, true, true, false),
            VenueReadiness::SignalOnly
        );
    }
}
