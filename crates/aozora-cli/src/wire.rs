use aozora::json::SCHEMA_VERSION;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Envelope<T> {
    schema_version: u32,
    data: T,
}

impl<T> Envelope<T> {
    pub(crate) const fn new(data: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            data,
        }
    }
}
