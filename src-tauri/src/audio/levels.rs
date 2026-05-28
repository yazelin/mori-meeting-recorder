//! Audio level computation — peak + RMS in dB, given to VU meter.
//!
//! Pure functions, easily testable.

const DB_FLOOR: f32 = -120.0;
const SILENCE_LINEAR: f32 = 1e-6;

/// Linear amplitude → dB. Input <= SILENCE_LINEAR clamps to DB_FLOOR.
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= SILENCE_LINEAR {
        DB_FLOOR
    } else {
        20.0 * linear.log10()
    }
}

/// Compute (peak_db, rms_db) from a batch of f32 samples (normalized to ±1.0).
/// Empty slice → (DB_FLOOR, DB_FLOOR).
pub fn compute_levels(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (DB_FLOOR, DB_FLOOR);
    }
    let mut peak_lin: f32 = 0.0;
    let mut sumsq: f64 = 0.0;
    for &s in samples {
        let abs = s.abs();
        if abs > peak_lin {
            peak_lin = abs;
        }
        sumsq += (s as f64) * (s as f64);
    }
    let rms_lin = (sumsq / samples.len() as f64).sqrt() as f32;
    (linear_to_db(peak_lin), linear_to_db(rms_lin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_to_db_full_scale_is_zero() {
        assert!((linear_to_db(1.0) - 0.0).abs() < 0.01);
    }

    #[test]
    fn linear_to_db_silence_floor() {
        // 1e-6 should clamp to ~-120 dB (avoid log(0))
        assert!((linear_to_db(1e-6) - (-120.0)).abs() < 0.5);
    }

    #[test]
    fn linear_to_db_half_scale() {
        // 0.5 → 20 * log10(0.5) ≈ -6.02 dB
        assert!((linear_to_db(0.5) - (-6.02)).abs() < 0.05);
    }

    #[test]
    fn compute_levels_empty_returns_silence() {
        let (peak, rms) = compute_levels(&[]);
        assert!(peak < -100.0);
        assert!(rms < -100.0);
    }

    #[test]
    fn compute_levels_full_scale_sine_approx() {
        // 1024 samples of sin near amplitude 1.0
        let samples: Vec<f32> = (0..1024)
            .map(|i| (i as f32 * 0.1).sin())
            .collect();
        let (peak, rms) = compute_levels(&samples);
        // sine peak ≈ 1.0 → 0 dB (allow 1 dB tolerance)
        assert!(peak.abs() < 1.0, "peak={peak}, expected ~0 dB");
        // sine RMS = 1/sqrt(2) ≈ 0.707 → -3.01 dB
        assert!((rms - (-3.01)).abs() < 0.5, "rms={rms}, expected ~-3 dB");
    }

    #[test]
    fn compute_levels_dc_offset_only_rms_zero_peak_matches() {
        // All 0.5 → peak = 0.5 → ~-6 dB, RMS also ~-6 dB
        let samples = vec![0.5_f32; 1000];
        let (peak, rms) = compute_levels(&samples);
        assert!((peak - (-6.02)).abs() < 0.1);
        assert!((rms - (-6.02)).abs() < 0.1);
    }
}
