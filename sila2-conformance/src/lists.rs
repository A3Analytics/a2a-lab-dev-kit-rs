//! ListDataTypeTest feature.

use tonic::{Request, Response, Status};

use crate::error;
use crate::support::{
    self, binary_bytes, boolean, date, integer, real, string, time_value, timestamp,
};
use crate::wire::sila2::org::silastandard::test::listdatatypetest::v1::data_type_test_structure::TestStructureStruct;
use crate::wire::sila2::org::silastandard::test::listdatatypetest::v1::list_data_type_test_server::ListDataTypeTest;
use crate::wire::sila2::org::silastandard::test::listdatatypetest::v1::{
    DataTypeTestStructure, EchoIntegerListParameters, EchoIntegerListResponses,
    EchoStringListParameters, EchoStringListResponses, EchoStructureListParameters,
    EchoStructureListResponses, GetEmptyStringListParameters, GetEmptyStringListResponses,
    GetIntegerListParameters, GetIntegerListResponses, GetStringListParameters,
    GetStringListResponses, GetStructureListParameters, GetStructureListResponses,
};

const MAX_STRING: usize = 2_097_152;
const LIST_STRING: &str =
    "org.silastandard/test/ListDataTypeTest/v1/Command/EchoStringList/Parameter/StringList";

#[derive(Clone, Copy, Default)]
pub struct Feature;

#[tonic::async_trait]
impl ListDataTypeTest for Feature {
    async fn echo_string_list(
        &self,
        request: Request<EchoStringListParameters>,
    ) -> Result<Response<EchoStringListResponses>, Status> {
        let values = request.into_inner().string_list;
        if values
            .iter()
            .any(|value| value.value.chars().count() > MAX_STRING)
        {
            return Err(error::validation(
                LIST_STRING,
                "String must not exceed 2²¹ characters",
            ));
        }
        Ok(Response::new(EchoStringListResponses {
            received_values: values,
        }))
    }

    async fn echo_integer_list(
        &self,
        request: Request<EchoIntegerListParameters>,
    ) -> Result<Response<EchoIntegerListResponses>, Status> {
        Ok(Response::new(EchoIntegerListResponses {
            received_values: request.into_inner().integer_list,
        }))
    }

    async fn echo_structure_list(
        &self,
        request: Request<EchoStructureListParameters>,
    ) -> Result<Response<EchoStructureListResponses>, Status> {
        Ok(Response::new(EchoStructureListResponses {
            received_values: request.into_inner().structure_list,
        }))
    }

    async fn get_empty_string_list(
        &self,
        _request: Request<GetEmptyStringListParameters>,
    ) -> Result<Response<GetEmptyStringListResponses>, Status> {
        Ok(Response::new(GetEmptyStringListResponses {
            empty_string_list: Vec::new(),
        }))
    }

    async fn get_string_list(
        &self,
        _request: Request<GetStringListParameters>,
    ) -> Result<Response<GetStringListResponses>, Status> {
        Ok(Response::new(GetStringListResponses {
            string_list: vec![string("SiLA 2"), string("is"), string("great")],
        }))
    }

    async fn get_integer_list(
        &self,
        _request: Request<GetIntegerListParameters>,
    ) -> Result<Response<GetIntegerListResponses>, Status> {
        Ok(Response::new(GetIntegerListResponses {
            integer_list: vec![integer(1), integer(2), integer(3)],
        }))
    }

    async fn get_structure_list(
        &self,
        _request: Request<GetStructureListParameters>,
    ) -> Result<Response<GetStructureListResponses>, Status> {
        Ok(Response::new(GetStructureListResponses {
            structure_list: vec![
                sample(1, true, 0),
                sample(2, false, 0),
                sample(3, true, 789),
            ],
        }))
    }
}

fn sample(index: i64, flag: bool, millisecond: u32) -> DataTypeTestStructure {
    let year = u32::try_from(2021 + index).unwrap_or(2022);
    let month = u32::try_from(7 + index).unwrap_or(8);
    let day = u32::try_from(4 + index).unwrap_or(5);
    let hour = u32::try_from(11 + index).unwrap_or(12);
    let minute = u32::try_from(33 + index).unwrap_or(34);
    let second = u32::try_from(55 + index).unwrap_or(56);
    DataTypeTestStructure {
        test_structure: Some(TestStructureStruct {
            string_type_value: Some(string(format!("SiLA2_Test_String_Value_{index}"))),
            integer_type_value: Some(integer(5123 + index)),
            real_type_value: Some(real(crate::support::TEST_REAL - 1.0 + i64_to_f64(index))),
            boolean_type_value: Some(boolean(flag)),
            binary_type_value: Some(binary_bytes(
                format!("Binary_String_Value_{index}").into_bytes(),
            )),
            date_type_value: Some(date(year, month, day, 2)),
            time_type_value: Some(time_value(hour, minute, second, millisecond, 2)),
            timestamp_type_value: Some(timestamp(
                year,
                month,
                day,
                hour,
                minute,
                second,
                millisecond,
                2,
            )),
            any_type_value: Some(support::basic_any(
                "String",
                support::embed_message(&string(format!("Any_Type_String_Value_{index}"))),
            )),
        }),
    }
}

fn i64_to_f64(value: i64) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(0))
}
