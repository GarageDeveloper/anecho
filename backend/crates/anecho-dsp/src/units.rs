//! Decibel and unit conversions.
//!
//! Conventions (see README): `dBFS` refers to a **peak** level (full-scale sine = 0 dBFS);
//! RMS quantities carry `_rms` in their name. Electrical units: `dBV` = 20·log10(Vrms / 1 V),
//! `dBu` = 20·log10(Vrms / 0.774597 V) (1 mW into 600 Ω), so `dBV = dBu − 2.2185`.
//!
//! `anecho-device` reports `Scale::Volts { dbv_offset }` meaning `dBV = dBFS_rms + offset`
//! for any RMS quantity (a level meter, a spectrum bin): [`dbfs_rms_to_dbv`].

/// Reference voltage of 0 dBu: √0.6 V.
pub const DBU_REF_VRMS: f64 = 0.774_596_669_241_483_4;
/// `dBV − dBu` for the same voltage.
pub const DBV_MINUS_DBU: f64 = -2.218_487_496_163_564;
/// Floor returned by [`db`] for non-positive amplitudes.
pub const DB_FLOOR: f64 = -400.0;

/// 20·log10(x) for an amplitude ratio, floored at [`DB_FLOOR`] for x ≤ 0.
pub fn db(x: f64) -> f64 {
    if x > 0.0 {
        (20.0 * x.log10()).max(DB_FLOOR)
    } else {
        DB_FLOOR
    }
}

/// 10·log10(p) for a power ratio, floored at [`DB_FLOOR`] for p ≤ 0.
pub fn power_db(p: f64) -> f64 {
    if p > 0.0 {
        (10.0 * p.log10()).max(DB_FLOOR)
    } else {
        DB_FLOOR
    }
}

/// Amplitude ratio of a dB value: 10^(dB/20).
pub fn from_db(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Power ratio of a dB value: 10^(dB/10).
pub fn power_from_db(db: f64) -> f64 {
    10f64.powf(db / 10.0)
}

/// RMS level (dBFS_rms) of a sine given its peak level (dBFS): −3.0103 dB.
pub fn sine_peak_to_rms_db(peak_dbfs: f64) -> f64 {
    peak_dbfs - 20.0 * std::f64::consts::SQRT_2.log10()
}

/// Peak level (dBFS) of a sine given its RMS level (dBFS_rms): +3.0103 dB.
pub fn sine_rms_to_peak_db(rms_dbfs: f64) -> f64 {
    rms_dbfs + 20.0 * std::f64::consts::SQRT_2.log10()
}

/// dBFS_rms of a linear RMS value (full scale = 1.0).
pub fn dbfs_rms(rms: f64) -> f64 {
    db(rms)
}

/// Apply a device's volt scale: `dBV = dBFS_rms + dbv_offset`.
pub fn dbfs_rms_to_dbv(dbfs_rms: f64, dbv_offset: f64) -> f64 {
    dbfs_rms + dbv_offset
}

pub fn dbv_to_vrms(dbv: f64) -> f64 {
    from_db(dbv)
}

pub fn vrms_to_dbv(vrms: f64) -> f64 {
    db(vrms)
}

pub fn dbu_to_dbv(dbu: f64) -> f64 {
    dbu + DBV_MINUS_DBU
}

pub fn dbv_to_dbu(dbv: f64) -> f64 {
    dbv - DBV_MINUS_DBU
}

pub fn dbu_to_vrms(dbu: f64) -> f64 {
    DBU_REF_VRMS * from_db(dbu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert!((db(1.0)).abs() < 1e-12);
        assert!((db(0.5) + 6.0206).abs() < 1e-3);
        assert_eq!(db(0.0), DB_FLOOR);
        assert!((from_db(-6.0206) - 0.5).abs() < 1e-4);
        assert!((dbu_to_dbv(0.0) + 2.2185).abs() < 1e-3);
        assert!((dbu_to_vrms(0.0) - 0.774597).abs() < 1e-6);
        assert!((dbv_to_vrms(0.0) - 1.0).abs() < 1e-12);
        assert!((sine_peak_to_rms_db(0.0) + 3.0103).abs() < 1e-3);
        assert!((dbfs_rms_to_dbv(-3.0103, 9.75) - 6.7397).abs() < 1e-3);
    }
}
