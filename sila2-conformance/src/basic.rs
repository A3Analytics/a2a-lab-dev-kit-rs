//! BasicDataTypesTest feature.

use tonic::{Request, Response, Status};

use crate::error;
use crate::support::{boolean, date, integer, real, string, time_value, timestamp};
use crate::wire::sila2::org::silastandard::test::basicdatatypestest::v1::basic_data_types_test_server::BasicDataTypesTest;
use crate::wire::sila2::org::silastandard::test::basicdatatypestest::v1::{
    EchoBooleanValueParameters, EchoBooleanValueResponses, EchoDateValueParameters, EchoDateValueResponses,
    EchoIntegerValueParameters, EchoIntegerValueResponses, EchoRealValueParameters, EchoRealValueResponses,
    EchoStringValueParameters, EchoStringValueResponses, EchoTimeValueParameters, EchoTimeValueResponses,
    EchoTimestampValueParameters, EchoTimestampValueResponses, GetBooleanValueParameters, GetBooleanValueResponses,
    GetDateValueParameters, GetDateValueResponses, GetIntegerValueParameters, GetIntegerValueResponses,
    GetRealValueParameters, GetRealValueResponses, GetStringValueParameters, GetStringValueResponses,
    GetTimeValueParameters, GetTimeValueResponses, GetTimestampValueParameters, GetTimestampValueResponses,
};
use crate::wire::sila2::org::silastandard::{self as fw, Date, Time, Timestamp};

const MAX_STRING: usize = 2_097_152;

#[derive(Clone, Copy, Default)]
pub struct Feature;

#[tonic::async_trait]
impl BasicDataTypesTest for Feature {
    async fn echo_string_value(
        &self,
        request: Request<EchoStringValueParameters>,
    ) -> Result<Response<EchoStringValueResponses>, Status> {
        let value = required(
            request.into_inner().string_value,
            "EchoStringValue",
            "StringValue",
        )?;
        if value.value.chars().count() > MAX_STRING {
            return Err(invalid(
                "EchoStringValue",
                "StringValue",
                "String must not exceed 2²¹ characters",
            ));
        }
        Ok(Response::new(EchoStringValueResponses {
            received_value: Some(value),
        }))
    }

    async fn echo_integer_value(
        &self,
        request: Request<EchoIntegerValueParameters>,
    ) -> Result<Response<EchoIntegerValueResponses>, Status> {
        let value = required(
            request.into_inner().integer_value,
            "EchoIntegerValue",
            "IntegerValue",
        )?;
        Ok(Response::new(EchoIntegerValueResponses {
            received_value: Some(value),
        }))
    }

    async fn echo_real_value(
        &self,
        request: Request<EchoRealValueParameters>,
    ) -> Result<Response<EchoRealValueResponses>, Status> {
        let value = required(
            request.into_inner().real_value,
            "EchoRealValue",
            "RealValue",
        )?;
        Ok(Response::new(EchoRealValueResponses {
            received_value: Some(value),
        }))
    }

    async fn echo_boolean_value(
        &self,
        request: Request<EchoBooleanValueParameters>,
    ) -> Result<Response<EchoBooleanValueResponses>, Status> {
        let value = required(
            request.into_inner().boolean_value,
            "EchoBooleanValue",
            "BooleanValue",
        )?;
        Ok(Response::new(EchoBooleanValueResponses {
            received_value: Some(value),
        }))
    }

    async fn echo_date_value(
        &self,
        request: Request<EchoDateValueParameters>,
    ) -> Result<Response<EchoDateValueResponses>, Status> {
        let value = required(
            request.into_inner().date_value,
            "EchoDateValue",
            "DateValue",
        )?;
        check_date(&value)?;
        Ok(Response::new(EchoDateValueResponses {
            received_value: Some(value),
        }))
    }

    async fn echo_time_value(
        &self,
        request: Request<EchoTimeValueParameters>,
    ) -> Result<Response<EchoTimeValueResponses>, Status> {
        let value = required(
            request.into_inner().time_value,
            "EchoTimeValue",
            "TimeValue",
        )?;
        check_time(&value)?;
        Ok(Response::new(EchoTimeValueResponses {
            received_value: Some(value),
        }))
    }

    async fn echo_timestamp_value(
        &self,
        request: Request<EchoTimestampValueParameters>,
    ) -> Result<Response<EchoTimestampValueResponses>, Status> {
        let value = required(
            request.into_inner().timestamp_value,
            "EchoTimestampValue",
            "TimestampValue",
        )?;
        check_timestamp(&value)?;
        Ok(Response::new(EchoTimestampValueResponses {
            received_value: Some(value),
        }))
    }

    async fn get_string_value(
        &self,
        _request: Request<GetStringValueParameters>,
    ) -> Result<Response<GetStringValueResponses>, Status> {
        Ok(Response::new(GetStringValueResponses {
            string_value: Some(string("SiLA2_Test_String_Value")),
        }))
    }

    async fn get_integer_value(
        &self,
        _request: Request<GetIntegerValueParameters>,
    ) -> Result<Response<GetIntegerValueResponses>, Status> {
        Ok(Response::new(GetIntegerValueResponses {
            integer_value: Some(integer(5124)),
        }))
    }

    async fn get_real_value(
        &self,
        _request: Request<GetRealValueParameters>,
    ) -> Result<Response<GetRealValueResponses>, Status> {
        Ok(Response::new(GetRealValueResponses {
            real_value: Some(real(crate::support::TEST_REAL)),
        }))
    }

    async fn get_boolean_value(
        &self,
        _request: Request<GetBooleanValueParameters>,
    ) -> Result<Response<GetBooleanValueResponses>, Status> {
        Ok(Response::new(GetBooleanValueResponses {
            boolean_value: Some(boolean(true)),
        }))
    }

    async fn get_date_value(
        &self,
        _request: Request<GetDateValueParameters>,
    ) -> Result<Response<GetDateValueResponses>, Status> {
        Ok(Response::new(GetDateValueResponses {
            date_value: Some(date(2022, 8, 5, 2)),
        }))
    }

    async fn get_time_value(
        &self,
        _request: Request<GetTimeValueParameters>,
    ) -> Result<Response<GetTimeValueResponses>, Status> {
        Ok(Response::new(GetTimeValueResponses {
            time_value: Some(time_value(12, 34, 56, 789, 2)),
        }))
    }

    async fn get_timestamp_value(
        &self,
        _request: Request<GetTimestampValueParameters>,
    ) -> Result<Response<GetTimestampValueResponses>, Status> {
        Ok(Response::new(GetTimestampValueResponses {
            timestamp_value: Some(timestamp(2022, 8, 5, 12, 34, 56, 789, 2)),
        }))
    }
}

fn required<T>(value: Option<T>, command: &str, parameter: &str) -> Result<T, Status> {
    value.ok_or_else(|| invalid(command, parameter, "Missing parameter"))
}

fn invalid(command: &str, parameter: &str, message: &str) -> Status {
    error::validation(
        &format!(
            "org.silastandard/test/BasicDataTypesTest/v1/Command/{command}/Parameter/{parameter}"
        ),
        message,
    )
}

fn check_date(value: &Date) -> Result<(), Status> {
    let zone = value
        .timezone
        .as_ref()
        .ok_or_else(|| invalid("EchoDateValue", "DateValue", "Missing timezone"))?;
    in_range(
        u64::from(value.year),
        1,
        9999,
        "EchoDateValue",
        "DateValue",
        "Year must be between 1 and 9999",
    )?;
    in_range(
        u64::from(value.month),
        1,
        12,
        "EchoDateValue",
        "DateValue",
        "Month must be between 1 and 12",
    )?;
    in_range(
        u64::from(value.day),
        1,
        31,
        "EchoDateValue",
        "DateValue",
        "Day must be between 1 and 31",
    )?;
    check_offset(zone, "EchoDateValue", "DateValue")
}

fn check_time(value: &Time) -> Result<(), Status> {
    let zone = value
        .timezone
        .as_ref()
        .ok_or_else(|| invalid("EchoTimeValue", "TimeValue", "Missing timezone"))?;
    in_range(
        u64::from(value.hour),
        0,
        23,
        "EchoTimeValue",
        "TimeValue",
        "Hour must be between 0 and 23",
    )?;
    in_range(
        u64::from(value.minute),
        0,
        59,
        "EchoTimeValue",
        "TimeValue",
        "Minute must be between 0 and 59",
    )?;
    in_range(
        u64::from(value.second),
        0,
        59,
        "EchoTimeValue",
        "TimeValue",
        "Second must be between 0 and 59",
    )?;
    in_range(
        u64::from(value.millisecond),
        0,
        999,
        "EchoTimeValue",
        "TimeValue",
        "Millisecond must be between 0 and 999",
    )?;
    check_offset(zone, "EchoTimeValue", "TimeValue")
}

fn check_timestamp(value: &Timestamp) -> Result<(), Status> {
    let zone = value
        .timezone
        .as_ref()
        .ok_or_else(|| invalid("EchoTimestampValue", "TimestampValue", "Missing timezone"))?;
    in_range(
        u64::from(value.year),
        1,
        9999,
        "EchoTimestampValue",
        "TimestampValue",
        "Year must be between 1 and 9999",
    )?;
    in_range(
        u64::from(value.month),
        1,
        12,
        "EchoTimestampValue",
        "TimestampValue",
        "Month must be between 1 and 12",
    )?;
    in_range(
        u64::from(value.day),
        1,
        31,
        "EchoTimestampValue",
        "TimestampValue",
        "Day must be between 1 and 31",
    )?;
    in_range(
        u64::from(value.hour),
        0,
        23,
        "EchoTimestampValue",
        "TimestampValue",
        "Hour must be between 0 and 23",
    )?;
    in_range(
        u64::from(value.minute),
        0,
        59,
        "EchoTimestampValue",
        "TimestampValue",
        "Minute must be between 0 and 59",
    )?;
    in_range(
        u64::from(value.second),
        0,
        59,
        "EchoTimestampValue",
        "TimestampValue",
        "Second must be between 0 and 59",
    )?;
    in_range(
        u64::from(value.millisecond),
        0,
        999,
        "EchoTimestampValue",
        "TimestampValue",
        "Millisecond must be between 0 and 999",
    )?;
    check_offset(zone, "EchoTimestampValue", "TimestampValue")
}

fn check_offset(zone: &fw::Timezone, command: &str, parameter: &str) -> Result<(), Status> {
    let offset = i64::from(zone.hours) * 60 + i64::from(zone.minutes);
    if !(-840..=840).contains(&offset) {
        return Err(invalid(
            command,
            parameter,
            "Timezone must be between -14:00 and +14:00",
        ));
    }
    in_range(
        u64::from(zone.minutes),
        0,
        59,
        command,
        parameter,
        "Timezone minutes must be between 0 and 59",
    )
}

fn in_range(
    value: u64,
    min: u64,
    max: u64,
    command: &str,
    parameter: &str,
    message: &str,
) -> Result<(), Status> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(invalid(command, parameter, message))
    }
}
