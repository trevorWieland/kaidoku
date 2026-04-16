use super::BYTES_PER_MIB;

pub(super) fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let midpoint = values.len() / 2;
    if values.len() % 2 == 0 {
        f64::midpoint(values[midpoint - 1], values[midpoint])
    } else {
        values[midpoint]
    }
}

pub(super) fn mad(values: &[f64], median_value: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mut deviations = values
        .iter()
        .map(|value| (value - median_value).abs())
        .collect::<Vec<f64>>();
    median(&mut deviations)
}

pub(super) fn round_metric(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub(super) fn bytes_to_mib(bytes: u64) -> f64 {
    let high = u32::try_from(bytes >> 32).expect("high u64 half fits u32");
    let low = u32::try_from(bytes & u64::from(u32::MAX)).expect("low u64 half fits u32");
    let bytes_f64 = f64::from(high) * 4_294_967_296.0 + f64::from(low);
    bytes_f64 / BYTES_PER_MIB
}
