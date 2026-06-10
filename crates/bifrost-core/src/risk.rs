//! Deterministic risk banding.
//!
//! The numeric score is computed elsewhere from weighted factors (plan §6); this
//! module only maps a score onto a Green/Amber/Red band. The LLM never touches
//! either the score or the band.

use crate::model::RiskBand;

/// Upper bound (exclusive) of the Green band.
pub const AMBER_THRESHOLD: i32 = 34;
/// Lower bound (inclusive) of the Red band.
pub const RED_THRESHOLD: i32 = 67;

/// Map a deterministic risk score (0–100) onto a band.
///
/// Mirrors the thresholds used by the portal so server- and client-derived
/// bands never disagree.
pub fn band_for_score(score: i32) -> RiskBand {
    if score >= RED_THRESHOLD {
        RiskBand::Red
    } else if score >= AMBER_THRESHOLD {
        RiskBand::Amber
    } else {
        RiskBand::Green
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_partition_the_score_range() {
        assert_eq!(band_for_score(0), RiskBand::Green);
        assert_eq!(band_for_score(33), RiskBand::Green);
        assert_eq!(band_for_score(34), RiskBand::Amber);
        assert_eq!(band_for_score(66), RiskBand::Amber);
        assert_eq!(band_for_score(67), RiskBand::Red);
        assert_eq!(band_for_score(100), RiskBand::Red);
    }
}
