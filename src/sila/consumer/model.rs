//! Owned SiLA Feature model. Wire encoding does not depend on another SiLA crate.

/// One published Feature Definition.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct FeatureModel {
    pub originator: String,
    pub category: String,
    pub identifier: String,
    pub major: u32,
    pub display_name: String,
    pub description: String,
    pub commands: Vec<CommandModel>,
    pub properties: Vec<PropertyModel>,
    pub metadata: Vec<MetadataModel>,
    pub data_types: Vec<DataTypeModel>,
    pub xml: String,
}

/// A command, observable or not.
#[derive(Debug, Clone)]
pub(crate) struct CommandModel {
    pub identifier: String,
    #[allow(dead_code)]
    pub display_name: String,
    #[allow(dead_code)]
    pub description: String,
    pub observable: bool,
    pub parameters: Vec<Element>,
    pub responses: Vec<Element>,
    pub intermediate: Vec<Element>,
}

/// A property. Observable properties are not lab tasks.
#[derive(Debug, Clone)]
pub(crate) struct PropertyModel {
    pub identifier: String,
    #[allow(dead_code)]
    pub display_name: String,
    #[allow(dead_code)]
    pub description: String,
    pub observable: bool,
    pub data_type: SilaType,
}

/// Client metadata declared by a feature.
#[derive(Debug, Clone)]
pub(crate) struct MetadataModel {
    pub identifier: String,
    pub data_type: SilaType,
}

/// A custom data type.
#[derive(Debug, Clone)]
pub(crate) struct DataTypeModel {
    pub identifier: String,
    pub data_type: SilaType,
}

/// A named parameter, response, or structure element.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Element {
    pub identifier: String,
    pub data_type: SilaType,
}

/// A SiLA data type.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum SilaType {
    Basic(Basic),
    List(Box<SilaType>),
    Structure(Vec<Element>),
    Reference(String),
    Constrained {
        inner: Box<SilaType>,
        constraints: Constraints,
    },
}

/// SiLA basic types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Basic {
    String,
    Integer,
    Real,
    Boolean,
    Binary,
    Date,
    Time,
    Timestamp,
    Any,
}

/// Constraints that affect JSON Schema. The wire shape is the inner type.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Constraints {
    pub unit: Option<String>,
    pub length: Option<u64>,
    pub min_length: Option<u64>,
    pub max_length: Option<u64>,
    pub pattern: Option<String>,
    pub fully_qualified_identifier: bool,
    pub minimum: Option<f64>,
    pub exclusive_minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub exclusive_maximum: Option<f64>,
    pub element_count: Option<u64>,
    pub min_elements: Option<u64>,
    pub max_elements: Option<u64>,
    pub enumeration: Vec<String>,
}

impl FeatureModel {
    /// Fully qualified feature identifier.
    pub(crate) fn fqi(&self) -> String {
        format!(
            "{}/{}/{}/v{}",
            self.originator, self.category, self.identifier, self.major
        )
    }

    /// Protobuf package for this feature.
    pub(crate) fn package(&self) -> String {
        format!(
            "sila2.{}.{}.{}.v{}",
            self.originator,
            self.category,
            self.identifier.to_lowercase(),
            self.major
        )
    }

    pub(crate) fn command(&self, name: &str) -> Option<&CommandModel> {
        self.commands
            .iter()
            .find(|command| command.identifier == name)
    }

    pub(crate) fn property(&self, name: &str) -> Option<&PropertyModel> {
        self.properties
            .iter()
            .find(|property| property.identifier == name)
    }

    pub(crate) fn metadata(&self, name: &str) -> Option<&MetadataModel> {
        self.metadata.iter().find(|item| item.identifier == name)
    }

    pub(crate) fn data_type(&self, name: &str) -> Option<&SilaType> {
        self.data_types
            .iter()
            .find(|item| item.identifier == name)
            .map(|item| &item.data_type)
    }
}

impl SilaType {
    /// The type a constraint does not change on the wire.
    pub(crate) fn effective(&self) -> &Self {
        match self {
            Self::Constrained { inner, .. } => inner.effective(),
            other => other,
        }
    }
}
