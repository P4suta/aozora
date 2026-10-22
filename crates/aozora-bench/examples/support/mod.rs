use std::fmt::Debug;

pub(crate) fn report_number<N>(value: N) -> f64
where
    N: TryInto<u128>,
    N::Error: Debug,
{
    value
        .try_into()
        .expect("report quantities are nonnegative integers")
        .to_string()
        .parse()
        .expect("u128 values fit the finite f64 range")
}
