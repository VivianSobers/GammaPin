//! Sign conventions for dealer gamma.
//!
//! | | w_c | w_p |
//! |---|---|---|
//! | S0 US convention | +1 | −1 |
//! | S1 writers W = FII + Pro (day d−1) | (CL_W − CS_W) / ΣCL | (PL_W − PS_W) / ΣPL |
//! | S2 other side of clients | −(CL_Client − CS_Client) / ΣCL | −(PL_Client − PS_Client) / ΣPL |
//!
//! Participant OI covers all NSE index options and is not per strike, so S1 and S2
//! apply one weight to every strike. This approximation is stated in PREREGISTRATION.md.

use crate::chain::Weights;

/// One row of NSE's fao_participant_oi file, index options only (contracts).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ParticipantOi {
    pub call_long: f64,
    pub call_short: f64,
    pub put_long: f64,
    pub put_short: f64,
}

impl std::ops::Add for ParticipantOi {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self {
            call_long: self.call_long + o.call_long,
            call_short: self.call_short + o.call_short,
            put_long: self.put_long + o.put_long,
            put_short: self.put_short + o.put_short,
        }
    }
}

impl Weights {
    /// S1: net positioning of FII + Pro, scaled by total long OI.
    pub fn s1(fii: ParticipantOi, pro: ParticipantOi, total: ParticipantOi) -> Self {
        let w = fii + pro;
        Self {
            w_c: (w.call_long - w.call_short) / total.call_long,
            w_p: (w.put_long - w.put_short) / total.put_long,
        }
    }

    /// S2: the opposite of clients' net positioning, scaled by total long OI.
    pub fn s2(client: ParticipantOi, total: ParticipantOi) -> Self {
        Self {
            w_c: -(client.call_long - client.call_short) / total.call_long,
            w_p: -(client.put_long - client.put_short) / total.put_long,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Index-option columns from fao_participant_oi_10102025.csv
    const CLIENT: ParticipantOi = ParticipantOi {
        call_long: 2_310_910.0,
        call_short: 2_456_521.0,
        put_long: 2_881_227.0,
        put_short: 3_206_449.0,
    };
    const FII: ParticipantOi = ParticipantOi {
        call_long: 467_200.0,
        call_short: 454_751.0,
        put_long: 705_720.0,
        put_short: 425_386.0,
    };
    const PRO: ParticipantOi = ParticipantOi {
        call_long: 989_413.0,
        call_short: 867_405.0,
        put_long: 1_265_831.0,
        put_short: 1_251_862.0,
    };
    const TOTAL: ParticipantOi = ParticipantOi {
        call_long: 3_778_676.0,
        call_short: 3_778_676.0,
        put_long: 4_883_697.0,
        put_short: 4_883_697.0,
    };

    #[test]
    fn s1_from_real_file() {
        let w = Weights::s1(FII, PRO, TOTAL);
        assert!((w.w_c - 134_457.0 / 3_778_676.0).abs() < 1e-12);
        assert!((w.w_p - 294_303.0 / 4_883_697.0).abs() < 1e-12);
    }

    #[test]
    fn s2_from_real_file() {
        let w = Weights::s2(CLIENT, TOTAL);
        assert!((w.w_c - 145_611.0 / 3_778_676.0).abs() < 1e-12);
        assert!((w.w_p - 325_222.0 / 4_883_697.0).abs() < 1e-12);
    }
}
