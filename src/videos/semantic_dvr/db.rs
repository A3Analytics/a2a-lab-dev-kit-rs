use crate::error::A2aLabError;

pub(crate) fn db_error(error: impl std::fmt::Display) -> A2aLabError {
    A2aLabError::unavailable(format!("semantic DVR database: {error}"))
}
