//! StructureDataTypeTest feature.

use tonic::{Request, Response, Status};

use crate::any_xml::is_valid_any_type_xml;
use crate::error;
use crate::support::{self, binary_bytes, boolean, date, integer, real, string, text_of, time_value, timestamp};
use crate::wire::sila2::org::silastandard::test::structuredatatypetest::v1::data_type_deep_structure::deep_structure_struct::MiddleStructureStruct;
use crate::wire::sila2::org::silastandard::test::structuredatatypetest::v1::data_type_deep_structure::deep_structure_struct::middle_structure_struct::InnerStructureStruct;
use crate::wire::sila2::org::silastandard::test::structuredatatypetest::v1::data_type_deep_structure::DeepStructureStruct;
use crate::wire::sila2::org::silastandard::test::structuredatatypetest::v1::data_type_test_structure::TestStructureStruct;
use crate::wire::sila2::org::silastandard::test::structuredatatypetest::v1::structure_data_type_test_server::StructureDataTypeTest;
use crate::wire::sila2::org::silastandard::test::structuredatatypetest::v1::{
    DataTypeDeepStructure, DataTypeTestStructure, EchoDeepStructureValueParameters, EchoDeepStructureValueResponses,
    EchoStructureValueParameters, EchoStructureValueResponses, GetDeepStructureValueParameters,
    GetDeepStructureValueResponses, GetStructureValueParameters, GetStructureValueResponses,
};
use crate::wire::sila2::org::silastandard::{Date, Time, Timestamp, Timezone};

const STRUCTURE: &str = "org.silastandard/test/StructureDataTypeTest/v1/Command/EchoStructureValue/Parameter/StructureValue";
const DEEP: &str = "org.silastandard/test/StructureDataTypeTest/v1/Command/EchoDeepStructureValue/Parameter/DeepStructureValue";
const MAX_STRING: usize = 2_097_152;

#[derive(Clone, Copy, Default)]
pub struct Feature;

#[tonic::async_trait]
impl StructureDataTypeTest for Feature {
    async fn echo_structure_value(
        &self,
        request: Request<EchoStructureValueParameters>,
    ) -> Result<Response<EchoStructureValueResponses>, Status> {
        let Some(value) = request.into_inner().structure_value else {
            return Err(error::validation(STRUCTURE, "Missing parameter"));
        };
        if let Some(message) = structure_error(&value) {
            return Err(error::validation(STRUCTURE, message));
        }
        Ok(Response::new(EchoStructureValueResponses {
            received_values: Some(value),
        }))
    }

    async fn echo_deep_structure_value(
        &self,
        request: Request<EchoDeepStructureValueParameters>,
    ) -> Result<Response<EchoDeepStructureValueResponses>, Status> {
        let Some(value) = request.into_inner().deep_structure_value else {
            return Err(error::validation(DEEP, "Missing parameter"));
        };
        if let Some(message) = deep_messages(&value) {
            return Err(error::validation(DEEP, message));
        }
        Ok(Response::new(EchoDeepStructureValueResponses {
            received_values: Some(value),
        }))
    }

    async fn get_structure_value(
        &self,
        _request: Request<GetStructureValueParameters>,
    ) -> Result<Response<GetStructureValueResponses>, Status> {
        Ok(Response::new(GetStructureValueResponses {
            structure_value: Some(example_structure()),
        }))
    }

    async fn get_deep_structure_value(
        &self,
        _request: Request<GetDeepStructureValueParameters>,
    ) -> Result<Response<GetDeepStructureValueResponses>, Status> {
        Ok(Response::new(GetDeepStructureValueResponses {
            deep_structure_value: Some(example_deep()),
        }))
    }
}

struct Snapshot {
    string_len: usize,
    date: Date,
    time: Time,
    timestamp: Timestamp,
    any_type: String,
}

fn structure_error(value: &DataTypeTestStructure) -> Option<&'static str> {
    let snapshot = snapshot(value.test_structure.as_ref());
    string_limit(snapshot.string_len)
        .or_else(|| {
            range_u32(
                snapshot.date.year,
                1,
                9999,
                "Year must be between 1 and 9999",
            )
        })
        .or_else(|| {
            range_u32(
                snapshot.timestamp.year,
                1,
                9999,
                "Year must be between 1 and 9999",
            )
        })
        .or_else(|| range_u32(snapshot.date.month, 1, 12, "Month must be between 1 and 12"))
        .or_else(|| {
            range_u32(
                snapshot.timestamp.month,
                1,
                12,
                "Month must be between 1 and 12",
            )
        })
        .or_else(|| range_u32(snapshot.date.day, 1, 31, "Day must be between 1 and 31"))
        .or_else(|| {
            range_u32(
                snapshot.timestamp.day,
                1,
                31,
                "Day must be between 1 and 31",
            )
        })
        .or_else(|| range_u32(snapshot.time.hour, 0, 23, "Hour must be between 0 and 23"))
        .or_else(|| {
            range_u32(
                snapshot.timestamp.hour,
                0,
                23,
                "Hour must be between 0 and 23",
            )
        })
        .or_else(|| {
            range_u32(
                snapshot.time.minute,
                0,
                59,
                "Minute must be between 0 and 59",
            )
        })
        .or_else(|| {
            range_u32(
                snapshot.timestamp.minute,
                0,
                59,
                "Minute must be between 0 and 59",
            )
        })
        .or_else(|| {
            range_u32(
                snapshot.time.second,
                0,
                59,
                "Second must be between 0 and 59",
            )
        })
        .or_else(|| {
            range_u32(
                snapshot.timestamp.second,
                0,
                59,
                "Second must be between 0 and 59",
            )
        })
        .or_else(|| {
            range_u32(
                snapshot.time.millisecond,
                0,
                999,
                "Millisecond must be between 0 and 999",
            )
        })
        .or_else(|| {
            range_u32(
                snapshot.timestamp.millisecond,
                0,
                999,
                "Millisecond must be between 0 and 999",
            )
        })
        .or_else(|| zone_hours(snapshot.date.timezone.as_ref()))
        .or_else(|| zone_hours(snapshot.time.timezone.as_ref()))
        .or_else(|| zone_hours(snapshot.timestamp.timezone.as_ref()))
        .or_else(|| zone_minutes(snapshot.date.timezone.as_ref()))
        .or_else(|| zone_minutes(snapshot.time.timezone.as_ref()))
        .or_else(|| zone_minutes(snapshot.timestamp.timezone.as_ref()))
        .or_else(|| any_error(&snapshot.any_type))
}

fn snapshot(value: Option<&TestStructureStruct>) -> Snapshot {
    let date = value
        .and_then(|item| item.date_type_value)
        .unwrap_or_default();
    let time = value
        .and_then(|item| item.time_type_value)
        .unwrap_or_default();
    let timestamp = value
        .and_then(|item| item.timestamp_type_value)
        .unwrap_or_default();
    Snapshot {
        string_len: text_of(value.and_then(|item| item.string_type_value.as_ref()))
            .chars()
            .count(),
        date,
        time,
        timestamp,
        any_type: value
            .and_then(|item| item.any_type_value.as_ref())
            .map(|item| item.r#type.clone())
            .unwrap_or_default(),
    }
}

fn string_limit(length: usize) -> Option<&'static str> {
    (length > MAX_STRING).then_some("String must not exceed 2²¹ characters")
}

fn zone_hours(zone: Option<&Timezone>) -> Option<&'static str> {
    let hours = zone.map_or(0, |item| item.hours);
    (!(-12..=14).contains(&hours)).then_some("Hours must be between -12 and 14")
}

fn zone_minutes(zone: Option<&Timezone>) -> Option<&'static str> {
    let minutes = zone.map_or(0, |item| item.minutes);
    (!((0..=59).contains(&minutes))).then_some("Minutes must be between 0 and 59")
}

fn any_error(type_xml: &str) -> Option<&'static str> {
    (!is_valid_any_type_xml(type_xml)).then_some(
        "The value of 'StructureValue.TestStructure.AnyTypeValue.type' does not adhere to the schema 'AnyTypeDataType.xsd'",
    )
}

fn range_u32(value: u32, min: u32, max: u32, message: &'static str) -> Option<&'static str> {
    (!(min..=max).contains(&value)).then_some(message)
}

fn deep_messages(value: &DataTypeDeepStructure) -> Option<&'static str> {
    if value.deep_structure.is_none() {
        return Some("Missing structure field DeepStructureValue.DeepStructure");
    }
    let outer = value.deep_structure.as_ref()?;
    if outer.outer_string_type_value.is_none() {
        return Some(
            "Missing structure field DeepStructureValue.DeepStructure.OuterStringTypeValue",
        );
    }
    if outer.outer_integer_type_value.is_none() {
        return Some(
            "Missing structure field DeepStructureValue.DeepStructure.OuterIntegerTypeValue",
        );
    }
    if outer.middle_structure.is_none() {
        return Some("Missing structure field DeepStructureValue.DeepStructure.MiddleStructure");
    }
    middle_messages(outer)
}

fn middle_messages(outer: &DeepStructureStruct) -> Option<&'static str> {
    let middle = outer.middle_structure.as_ref()?;
    if middle.middle_string_type_value.is_none() {
        return Some(
            "Missing structure field DeepStructureValue.DeepStructure.MiddleStructure.MiddleStringTypeValue",
        );
    }
    if middle.middle_integer_type_value.is_none() {
        return Some(
            "Missing structure field DeepStructureValue.DeepStructure.MiddleStructure.MiddleIntegerTypeValue",
        );
    }
    if middle.inner_structure.is_none() {
        return Some(
            "Missing structure field DeepStructureValue.DeepStructure.MiddleStructure.InnerStructure",
        );
    }
    inner_messages(middle).or_else(|| string_lengths(outer, middle))
}

fn inner_messages(middle: &MiddleStructureStruct) -> Option<&'static str> {
    let inner = middle.inner_structure.as_ref()?;
    if inner.inner_string_type_value.is_none() {
        return Some(
            "Missing structure field DeepStructureValue.DeepStructure.MiddleStructure.InnerStructure.InnerStringTypeValue",
        );
    }
    if inner.inner_integer_type_value.is_none() {
        return Some(
            "Missing structure field DeepStructureValue.DeepStructure.MiddleStructure.InnerStructure.InnerIntegerTypeValue",
        );
    }
    None
}

fn string_lengths(
    outer: &DeepStructureStruct,
    middle: &MiddleStructureStruct,
) -> Option<&'static str> {
    let outer_len = text_of(outer.outer_string_type_value.as_ref())
        .chars()
        .count();
    if outer_len > MAX_STRING {
        return Some(
            "String must not exceed 2²¹ characters (element DeepStructureValue.DeepStructure.OuterStringTypeValue)",
        );
    }
    let middle_len = text_of(middle.middle_string_type_value.as_ref())
        .chars()
        .count();
    if middle_len > MAX_STRING {
        return Some(
            "String must not exceed 2²¹ characters (element DeepStructureValue.DeepStructure.MiddleStructure.MiddleStringTypeValue)",
        );
    }
    let inner_len = text_of(
        middle
            .inner_structure
            .as_ref()
            .and_then(|inner| inner.inner_string_type_value.as_ref()),
    )
    .chars()
    .count();
    (inner_len > MAX_STRING).then_some(
        "String must not exceed 2²¹ characters (element DeepStructureValue.DeepStructure.MiddleStructure.InnerStructure.InnerStringTypeValue)",
    )
}

fn example_structure() -> DataTypeTestStructure {
    DataTypeTestStructure {
        test_structure: Some(TestStructureStruct {
            string_type_value: Some(string("SiLA2_Test_String_Value")),
            integer_type_value: Some(integer(5124)),
            real_type_value: Some(real(crate::support::TEST_REAL)),
            boolean_type_value: Some(boolean(true)),
            binary_type_value: Some(binary_bytes(b"SiLA2_Binary_String_Value".to_vec())),
            date_type_value: Some(date(2022, 8, 5, 2)),
            time_type_value: Some(time_value(12, 34, 56, 789, 2)),
            timestamp_type_value: Some(timestamp(2022, 8, 5, 12, 34, 56, 789, 2)),
            any_type_value: Some(support::basic_any(
                "String",
                support::embed_message(&string("SiLA2_Any_Type_String_Value")),
            )),
        }),
    }
}

fn example_deep() -> DataTypeDeepStructure {
    DataTypeDeepStructure {
        deep_structure: Some(DeepStructureStruct {
            outer_string_type_value: Some(string("Outer_Test_String")),
            outer_integer_type_value: Some(integer(1111)),
            middle_structure: Some(MiddleStructureStruct {
                middle_string_type_value: Some(string("Middle_Test_String")),
                middle_integer_type_value: Some(integer(2222)),
                inner_structure: Some(InnerStructureStruct {
                    inner_string_type_value: Some(string("Inner_Test_String")),
                    inner_integer_type_value: Some(integer(3333)),
                }),
            }),
        }),
    }
}
