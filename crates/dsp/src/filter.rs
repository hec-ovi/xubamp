use std::f32::consts::PI;

#[derive(Debug, Clone, Copy)]
pub(crate) enum FilterKind {
    LowShelf,
    Peaking,
    HighShelf,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    pub(crate) const BYPASS: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// RBJ audio-EQ-cookbook coefficients. Shelf slope and peaking Q both use 1.0, matching the
    /// default Web Audio BiquadFilter parameters Webamp relies on.
    pub(crate) fn design(kind: FilterKind, sample_rate: f32, frequency: f32, gain_db: f32) -> Self {
        if gain_db.abs() < f32::EPSILON
            || sample_rate <= 0.0
            || frequency <= 0.0
            || frequency >= sample_rate * 0.5
        {
            return Self::BYPASS;
        }
        let a = 10.0f32.powf(gain_db / 40.0);
        let omega = 2.0 * PI * frequency / sample_rate;
        let cos = omega.cos();
        let sin = omega.sin();

        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::Peaking => {
                let alpha = sin / 2.0; // Q = 1
                (
                    1.0 + alpha * a,
                    -2.0 * cos,
                    1.0 - alpha * a,
                    1.0 + alpha / a,
                    -2.0 * cos,
                    1.0 - alpha / a,
                )
            }
            FilterKind::LowShelf => {
                let alpha = sin / 2.0 * 2.0f32.sqrt(); // shelf slope S = 1
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha),
                    (a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha,
                )
            }
            FilterKind::HighShelf => {
                let alpha = sin / 2.0 * 2.0f32.sqrt(); // shelf slope S = 1
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha),
                    (a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha,
                )
            }
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

/// Transposed direct-form II. The two state words are retained when coefficients change.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Biquad {
    coefficients: Coefficients,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub(crate) const fn bypass() -> Self {
        Self {
            coefficients: Coefficients::BYPASS,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub(crate) fn set_coefficients(&mut self, coefficients: Coefficients) {
        self.coefficients = coefficients;
    }

    pub(crate) fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    pub(crate) fn process(&mut self, input: f32) -> f32 {
        let c = self.coefficients;
        let output = c.b0 * input + self.z1;
        self.z1 = c.b1 * input - c.a1 * output + self.z2;
        self.z2 = c.b2 * input - c.a2 * output;
        output
    }
}
