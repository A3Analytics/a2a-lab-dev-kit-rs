//! AnyTypeTest feature.

use prost::Message;
use tonic::{Request, Response, Status};

use crate::any_xml::is_valid_any_type_xml;
use crate::error;
use crate::support::{
    any_message, basic_any, binary_bytes, boolean, date, embed_message, embed_repeated, integer,
    real, string, time_value, timestamp,
};
use crate::wire::sila2::org::silastandard::Integer;
use crate::wire::sila2::org::silastandard::test::anytypetest::v1::any_type_test_server::AnyTypeTest;
use crate::wire::sila2::org::silastandard::test::anytypetest::v1::{
    GetAnyTypeBinaryValueParameters, GetAnyTypeBinaryValueResponses,
    GetAnyTypeBooleanValueParameters, GetAnyTypeBooleanValueResponses,
    GetAnyTypeDateValueParameters, GetAnyTypeDateValueResponses, GetAnyTypeIntegerValueParameters,
    GetAnyTypeIntegerValueResponses, GetAnyTypeListValueParameters, GetAnyTypeListValueResponses,
    GetAnyTypeRealValueParameters, GetAnyTypeRealValueResponses, GetAnyTypeStringValueParameters,
    GetAnyTypeStringValueResponses, GetAnyTypeStructureValueParameters,
    GetAnyTypeStructureValueResponses, GetAnyTypeTimeValueParameters, GetAnyTypeTimeValueResponses,
    GetAnyTypeTimestampValueParameters, GetAnyTypeTimestampValueResponses,
    SetAnyTypeValueParameters, SetAnyTypeValueResponses,
};

const ANY_PARAMETER: &str =
    "org.silastandard/test/AnyTypeTest/v1/Command/SetAnyTypeValue/Parameter/AnyTypeValue";
const ANY_MESSAGE: &str =
    "The value of 'AnyTypeValue.type' does not adhere to the schema 'AnyTypeDataType.xsd'";
const STRING_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><DataType xmlns=\"http://www.sila-standard.org\"><Basic>String</Basic></DataType>";
const LIST_XML: &str = "<DataType xmlns=\"http://www.sila-standard.org\"><List><DataType><Basic>String</Basic></DataType></List></DataType>";
const STRUCTURE_XML: &str = "<DataType xmlns=\"http://www.sila-standard.org\"><Structure><Element><Identifier>StringTypeValue</Identifier><DisplayName>String Type Value</DisplayName><Description>A string value.</Description><DataType><Basic>String</Basic></DataType></Element><Element><Identifier>IntegerTypeValue</Identifier><DisplayName>Integer Type Value</DisplayName><Description>An integer value.</Description><DataType><Basic>Integer</Basic></DataType></Element><Element><Identifier>DateTypeValue</Identifier><DisplayName>Date Type Value</DisplayName><Description>A date value.</Description><DataType><Basic>Date</Basic></DataType></Element></Structure></DataType>";

#[derive(Clone, Copy, Default)]
pub struct Feature;

#[tonic::async_trait]
impl AnyTypeTest for Feature {
    async fn set_any_type_value(
        &self,
        request: Request<SetAnyTypeValueParameters>,
    ) -> Result<Response<SetAnyTypeValueResponses>, Status> {
        let any = request.into_inner().any_type_value.unwrap_or_default();
        if !is_valid_any_type_xml(&any.r#type) {
            return Err(error::validation(ANY_PARAMETER, ANY_MESSAGE));
        }
        Ok(Response::new(SetAnyTypeValueResponses {
            received_any_type: Some(string(any.r#type.clone())),
            received_value: Some(any),
        }))
    }

    async fn get_any_type_string_value(
        &self,
        _request: Request<GetAnyTypeStringValueParameters>,
    ) -> Result<Response<GetAnyTypeStringValueResponses>, Status> {
        Ok(Response::new(GetAnyTypeStringValueResponses {
            any_type_string_value: Some(any_message(
                STRING_XML,
                embed_message(&string("SiLA_Any_type_of_String_type")),
            )),
        }))
    }

    async fn get_any_type_integer_value(
        &self,
        _request: Request<GetAnyTypeIntegerValueParameters>,
    ) -> Result<Response<GetAnyTypeIntegerValueResponses>, Status> {
        Ok(Response::new(GetAnyTypeIntegerValueResponses {
            any_type_integer_value: Some(basic_any("Integer", embed_message(&integer(5124)))),
        }))
    }

    async fn get_any_type_real_value(
        &self,
        _request: Request<GetAnyTypeRealValueParameters>,
    ) -> Result<Response<GetAnyTypeRealValueResponses>, Status> {
        Ok(Response::new(GetAnyTypeRealValueResponses {
            any_type_real_value: Some(basic_any(
                "Real",
                embed_message(&real(crate::support::TEST_REAL)),
            )),
        }))
    }

    async fn get_any_type_boolean_value(
        &self,
        _request: Request<GetAnyTypeBooleanValueParameters>,
    ) -> Result<Response<GetAnyTypeBooleanValueResponses>, Status> {
        Ok(Response::new(GetAnyTypeBooleanValueResponses {
            any_type_boolean_value: Some(basic_any("Boolean", embed_message(&boolean(true)))),
        }))
    }

    async fn get_any_type_binary_value(
        &self,
        _request: Request<GetAnyTypeBinaryValueParameters>,
    ) -> Result<Response<GetAnyTypeBinaryValueResponses>, Status> {
        Ok(Response::new(GetAnyTypeBinaryValueResponses {
            any_type_binary_value: Some(basic_any(
                "Binary",
                embed_message(&binary_bytes(b"SiLA_Any_type_of_Binary_type".to_vec())),
            )),
        }))
    }

    async fn get_any_type_date_value(
        &self,
        _request: Request<GetAnyTypeDateValueParameters>,
    ) -> Result<Response<GetAnyTypeDateValueResponses>, Status> {
        Ok(Response::new(GetAnyTypeDateValueResponses {
            any_type_date_value: Some(basic_any("Date", embed_message(&date(2022, 8, 5, 2)))),
        }))
    }

    async fn get_any_type_time_value(
        &self,
        _request: Request<GetAnyTypeTimeValueParameters>,
    ) -> Result<Response<GetAnyTypeTimeValueResponses>, Status> {
        Ok(Response::new(GetAnyTypeTimeValueResponses {
            any_type_time_value: Some(basic_any(
                "Time",
                embed_message(&time_value(12, 34, 56, 789, 2)),
            )),
        }))
    }

    async fn get_any_type_timestamp_value(
        &self,
        _request: Request<GetAnyTypeTimestampValueParameters>,
    ) -> Result<Response<GetAnyTypeTimestampValueResponses>, Status> {
        Ok(Response::new(GetAnyTypeTimestampValueResponses {
            any_type_timestamp_value: Some(basic_any(
                "Timestamp",
                embed_message(&timestamp(2022, 8, 5, 12, 34, 56, 789, 2)),
            )),
        }))
    }

    async fn get_any_type_list_value(
        &self,
        _request: Request<GetAnyTypeListValueParameters>,
    ) -> Result<Response<GetAnyTypeListValueResponses>, Status> {
        let parts =
            ["SiLA 2", "Any", "Type", "String", "List"].map(|value| encoded(&string(value)));
        Ok(Response::new(GetAnyTypeListValueResponses {
            any_type_list_value: Some(any_message(LIST_XML, embed_repeated(&parts))),
        }))
    }

    async fn get_any_type_structure_value(
        &self,
        _request: Request<GetAnyTypeStructureValueParameters>,
    ) -> Result<Response<GetAnyTypeStructureValueResponses>, Status> {
        Ok(Response::new(GetAnyTypeStructureValueResponses {
            any_type_structure_value: Some(any_message(STRUCTURE_XML, structure_payload())),
        }))
    }
}

fn encoded(message: &impl Message) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = message.encode(&mut bytes);
    bytes
}

fn structure_payload() -> Vec<u8> {
    let mut integer_bytes = Vec::new();
    let _ = Integer { value: 83_737_665 }.encode(&mut integer_bytes);
    let mut date_bytes = Vec::new();
    let _ = date(2022, 8, 5, 2).encode(&mut date_bytes);
    let mut payload = vec![0x0a, 0x26, 0x0a, 0x10, 0x0a, 0x0e];
    payload.extend_from_slice(b"A String value");
    payload.extend_from_slice(&[0x12, 0x05]);
    payload.extend(integer_bytes);
    payload.extend_from_slice(&[0x1a, 0x0b]);
    payload.extend(date_bytes);
    payload
}
