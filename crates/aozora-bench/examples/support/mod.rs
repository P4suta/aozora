use std::fmt::Debug;

#[expect(
    clippy::expect_used,
    reason = "report inputs are nonnegative integers and u128 values fit the finite f64 range"
)]
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
