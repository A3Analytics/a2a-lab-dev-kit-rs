//! Optional semantic search hits and Jev decision history for the Videos reader.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::{Map, Value};

use crate::codec::{self, Exchange, Outcome, ProtocolError, Reply};
use crate::inventory::Fault;
use crate::workbench::report;

pub const SEARCH_LIMIT: u32 = 10;
pub const HISTORY_LIMIT: u32 = 20;
pub const SEARCH_PAGE_SIZE: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiPhase {
    Unknown,
    Absent,
    Ready,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityState {
    pub phase: ApiPhase,
    pub alert: String,
}

impl CapabilityState {
    pub fn new() -> Self {
        Self {
            phase: ApiPhase::Unknown,
            alert: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenancePick {
    pub model_space: Value,
    pub description_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageBanner {
    pub pending: u64,
    pub ready: u64,
    pub failed: u64,
    pub expired: u64,
    pub incomplete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitThumb {
    pub url: String,
    pub withheld: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub frame_id: String,
    pub captured_at: String,
    pub captured_us: i64,
    pub description: String,
    pub description_provider: String,
    pub description_model: String,
    pub retrieval_score: String,
    pub source_id: String,
    pub source_label: String,
    pub index_state: String,
    pub media_state: String,
    pub media_reason: String,
    /// Operator-safe frame metadata for display.
    pub frame_ref: Value,
    /// Full frame revision for same-origin decision history requests.
    pub command_frame: Value,
    pub thumb: Option<HitThumb>,
    pub decisions: Vec<DecisionRow>,
    pub authorized_assessments: bool,
}

impl SearchHit {
    pub fn media_blocked(&self) -> bool {
        self.media_state == "unavailable" || self.index_state == "expired"
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionRow {
    pub decision_id: String,
    pub predicate: String,
    pub subject: String,
    pub outcome: String,
    pub coverage: String,
    pub producer: String,
    pub model_revision: String,
    pub probability: String,
    pub confidence: String,
    pub roi: String,
    pub evaluated_at: String,
    pub evidence_state: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTicket {
    pub generation: u32,
    pub source_ids: Vec<String>,
    pub text: String,
    pub range_start: Option<String>,
    pub range_end: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hub {
    pub generation: u32,
    pub search_capabilities: CapabilityState,
    pub decision_capabilities: CapabilityState,
    pub provenance: Option<ProvenancePick>,
    pub coverage: Option<CoverageBanner>,
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub incomplete_indexing: bool,
    pub selected: usize,
    pub result_page: usize,
    pub decisions: Vec<DecisionRow>,
    pub decision_phase: ApiPhase,
    pub decision_alert: String,
    pub search_loading: bool,
    pub search_fault: String,
    pub expected: Option<SearchTicket>,
}

impl Hub {
    pub fn new() -> Self {
        Self {
            generation: 0,
            search_capabilities: CapabilityState::new(),
            decision_capabilities: CapabilityState::new(),
            provenance: None,
            coverage: None,
            query: String::new(),
            hits: Vec::new(),
            incomplete_indexing: false,
            selected: usize::MAX,
            result_page: 0,
            decisions: Vec::new(),
            decision_phase: ApiPhase::Unknown,
            decision_alert: String::new(),
            search_loading: false,
            search_fault: String::new(),
            expected: None,
        }
    }

    pub fn semantic_enabled(&self) -> bool {
        self.search_capabilities.phase == ApiPhase::Ready
    }

    pub fn decisions_enabled(&self) -> bool {
        self.decision_capabilities.phase == ApiPhase::Ready
    }

    pub fn selected_hit(&self) -> Option<&SearchHit> {
        self.hits.get(self.selected)
    }

    pub fn note_source(&mut self) -> u32 {
        self.generation = self.generation.wrapping_add(1);
        self.search_capabilities = CapabilityState::new();
        self.decision_capabilities = CapabilityState::new();
        self.provenance = None;
        self.coverage = None;
        self.hits.clear();
        self.incomplete_indexing = false;
        self.selected = usize::MAX;
        self.result_page = 0;
        self.decisions.clear();
        self.decision_phase = ApiPhase::Unknown;
        self.decision_alert.clear();
        self.search_loading = false;
        self.search_fault.clear();
        self.expected = None;
        self.generation
    }

    pub fn begin_search(
        &mut self,
        source_ids: &[String],
        text: &str,
        range_start: Option<&str>,
        range_end: Option<&str>,
    ) -> Option<SearchTicket> {
        let text = text.trim();
        if !self.semantic_enabled() || source_ids.is_empty() || text.is_empty() {
            return None;
        }
        let mut source_ids = source_ids.to_vec();
        source_ids.sort();
        source_ids.dedup();
        let ticket = SearchTicket {
            generation: self.generation,
            source_ids,
            text: text.to_owned(),
            range_start: range_start.map(str::to_owned),
            range_end: range_end.map(str::to_owned),
        };
        self.search_loading = true;
        self.search_fault.clear();
        self.expected = Some(ticket.clone());
        Some(ticket)
    }

    pub fn accepts_search(
        &mut self,
        ticket: &SearchTicket,
        hits: Vec<SearchHit>,
        incomplete: bool,
        fault: Option<String>,
    ) -> bool {
        if self.expected.as_ref() != Some(ticket) {
            return false;
        }
        self.search_loading = false;
        self.expected = None;
        if let Some(message) = fault {
            self.search_fault = message;
            self.hits.clear();
            self.incomplete_indexing = false;
            self.selected = usize::MAX;
            self.decisions.clear();
            return true;
        }
        self.search_fault.clear();
        self.hits = hits;
        self.incomplete_indexing = incomplete;
        self.selected = if self.hits.is_empty() { usize::MAX } else { 0 };
        self.result_page = 0;
        if self.selected < self.hits.len() {
            self.apply_decisions_from_hit(self.selected);
        } else {
            self.decisions.clear();
            self.decision_phase = ApiPhase::Unknown;
        }
        true
    }

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.hits.len() {
            return false;
        }
        if self.selected == index {
            return false;
        }
        self.selected = index;
        self.apply_decisions_from_hit(index);
        true
    }

    pub fn apply_decisions_from_hit(&mut self, index: usize) {
        let Some(hit) = self.hits.get(index) else {
            self.decisions.clear();
            self.decision_phase = ApiPhase::Unknown;
            self.decision_alert.clear();
            return;
        };
        self.decisions = hit.decisions.clone();
        self.decision_alert.clear();
        self.decision_phase = if !self.decisions_enabled() {
            ApiPhase::Absent
        } else if !hit.authorized_assessments {
            ApiPhase::Unknown
        } else {
            ApiPhase::Ready
        };
    }
}

pub fn semantic_capabilities_command() -> Value {
    serde_json::json!({
        "operation": "get_semantic_search_capabilities",
        "params": {}
    })
}

pub fn decision_capabilities_command() -> Value {
    serde_json::json!({
        "operation": "get_frame_decision_capabilities",
        "params": {}
    })
}

pub fn coverage_command(
    source_id: &str,
    range: Option<(&str, &str)>,
    provenance: &ProvenancePick,
) -> Value {
    let mut scope = serde_json::json!({ "source_id": source_id });
    if let Some((start, end)) = range {
        scope["range"] = serde_json::json!({ "start": start, "end": end });
    }
    serde_json::json!({
        "operation": "get_semantic_indexing_coverage",
        "params": {
            "scope": scope,
            "provenance": {
                "model_space": provenance.model_space,
                "description_generation": provenance.description_generation
            }
        }
    })
}

pub fn search_command(
    source_id: &str,
    range: Option<(&str, &str)>,
    text: &str,
    provenance: &ProvenancePick,
) -> Value {
    let mut scope = serde_json::json!({ "source_id": source_id });
    if let Some((start, end)) = range {
        scope["range"] = serde_json::json!({ "start": start, "end": end });
    }
    serde_json::json!({
        "operation": "search_semantic_frames",
        "params": {
            "scope": scope,
            "query": { "kind": "text", "text": text },
            "provenance": {
                "model_space": provenance.model_space,
                "description_generation": provenance.description_generation
            },
            "limits": {
                "top_k": SEARCH_LIMIT,
                "candidate_limit": 1000,
                "max_neighbor_frames": 5
            },
            "page": { "limit": SEARCH_LIMIT }
        }
    })
}

pub fn history_command(source_id: &str, start: &str, end: &str, frame: &Value) -> Value {
    serde_json::json!({
        "operation": "read_frame_decision_history",
        "params": {
            "scope": {
                "source_id": source_id,
                "range": { "start": start, "end": end },
                "frame": frame
            },
            "history_policy": "full_history",
            "limits": { "max_rows": HISTORY_LIMIT },
            "page": { "limit": HISTORY_LIMIT }
        }
    })
}

pub fn interpret_semantic_registered(reply: &Reply, secret: Option<&str>) -> Result<bool, Fault> {
    let (_, registered) = interpret_semantic_capabilities(reply, secret)?;
    Ok(registered)
}

/// Parses semantic search registration and server-selected provenance.
pub fn interpret_semantic_capabilities(
    reply: &Reply,
    secret: Option<&str>,
) -> Result<(Option<ProvenancePick>, bool), Fault> {
    let payload = one_payload(&decode(reply, secret)?)?;
    let value: Value = serde_json::from_str(&payload).map_err(|_| Fault::Malformed)?;
    let result = owned_result(&value, "get_semantic_search_capabilities")?;
    let registered = result
        .get("registered")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !registered {
        return Ok((None, false));
    }
    let active = result.get("active_provenance").ok_or(Fault::Malformed)?;
    Ok((Some(parse_provenance_pick(active)?), true))
}

fn parse_provenance_pick(value: &Value) -> Result<ProvenancePick, Fault> {
    let object = value.as_object().ok_or(Fault::Malformed)?;
    let model_space = object
        .get("model_space")
        .map(normalize_wire_numbers)
        .ok_or(Fault::Malformed)?;
    let description_generation =
        wire_u64(object.get("description_generation")).ok_or(Fault::Malformed)?;
    Ok(ProvenancePick {
        model_space,
        description_generation,
    })
}

/// A2A wire normalization may emit whole-number fields as floats (for example `1.0`).
fn wire_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if let Some(number) = value.as_u64() {
        return (number > 0).then_some(number);
    }
    if let Some(number) = value.as_i64().and_then(|v| u64::try_from(v).ok()) {
        return (number > 0).then_some(number);
    }
    let number = value.as_f64()?;
    if !(0.0..=f64::from(u32::MAX)).contains(&number) || number.fract() != 0.0 {
        return None;
    }
    let rounded = number.round() as u64;
    (rounded > 0).then_some(rounded)
}

fn normalize_wire_numbers(value: &Value) -> Value {
    match value {
        Value::Number(number) => whole_number_value(number),
        Value::Array(items) => {
            Value::Array(items.iter().map(normalize_wire_numbers).collect::<Vec<_>>())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), normalize_wire_numbers(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn whole_number_value(number: &serde_json::Number) -> Value {
    if number.as_u64().is_some() || number.as_i64().is_some() {
        return Value::Number(number.clone());
    }
    let Some(float) = number.as_f64() else {
        return Value::Number(number.clone());
    };
    if !(0.0..=f64::from(u32::MAX)).contains(&float) || float.fract() != 0.0 {
        return Value::Number(number.clone());
    }
    Value::Number(serde_json::Number::from(float.round() as u64))
}

pub fn interpret_decision_registered(reply: &Reply, secret: Option<&str>) -> Result<bool, Fault> {
    let payload = one_payload(&decode(reply, secret)?)?;
    let value: Value = serde_json::from_str(&payload).map_err(|_| Fault::Malformed)?;
    let result = owned_result(&value, "get_frame_decision_capabilities")?;
    Ok(result
        .get("registered")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

pub fn interpret_coverage(
    reply: &Reply,
    secret: Option<&str>,
) -> Result<(ProvenancePick, CoverageBanner), Fault> {
    let payload = one_payload(&decode(reply, secret)?)?;
    let value: Value = serde_json::from_str(&payload).map_err(|_| Fault::Malformed)?;
    let result = owned_result(&value, "get_semantic_indexing_coverage")?;
    let model_space = result.get("model_space").cloned().ok_or(Fault::Malformed)?;
    let generation = wire_u64(model_space.get("generation")).unwrap_or(1);
    let counts = result.get("counts").and_then(Value::as_object);
    let banner = CoverageBanner {
        pending: counts
            .and_then(|object| object.get("pending"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        ready: counts
            .and_then(|object| object.get("ready"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        failed: counts
            .and_then(|object| object.get("failed"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        expired: counts
            .and_then(|object| object.get("expired"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        incomplete: result
            .get("scope_complete")
            .and_then(Value::as_bool)
            .is_some_and(|complete| !complete)
            || counts
                .and_then(|object| object.get("pending"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0,
    };
    Ok((
        ProvenancePick {
            model_space,
            description_generation: generation,
        },
        banner,
    ))
}

pub fn interpret_search(
    reply: &Reply,
    secret: Option<&str>,
) -> Result<(Vec<SearchHit>, bool), Fault> {
    let payload = one_payload(&decode(reply, secret)?)?;
    let value: Value = serde_json::from_str(&payload).map_err(|_| Fault::Malformed)?;
    let result = owned_result(&value, "search_semantic_frames")?;
    let incomplete = result
        .get("incomplete_indexing")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let hits = result
        .get("hits")
        .and_then(Value::as_object)
        .and_then(|object| object.get("items"))
        .and_then(Value::as_array)
        .ok_or(Fault::Malformed)?;
    let mut parsed = Vec::new();
    for item in hits {
        if let Ok(hit) = parse_hit(item, secret) {
            parsed.push(hit);
        }
    }
    Ok((parsed, incomplete))
}

pub fn interpret_history(reply: &Reply, secret: Option<&str>) -> Result<Vec<DecisionRow>, Fault> {
    let payload = one_payload(&decode(reply, secret)?)?;
    let value: Value = serde_json::from_str(&payload).map_err(|_| Fault::Malformed)?;
    let result = owned_result(&value, "read_frame_decision_history")?;
    let items = result
        .get("entries")
        .and_then(Value::as_object)
        .and_then(|object| object.get("items"))
        .and_then(Value::as_array)
        .ok_or(Fault::Malformed)?;
    Ok(items
        .iter()
        .filter_map(|item| parse_decision(item, secret).ok())
        .collect())
}

pub fn thumb_from_frame(data: &str, media_type: &str, secret: Option<&str>) -> Option<HitThumb> {
    let preview = report::preview_for(media_type, data);
    let url = preview
        .preview_url
        .as_ref()
        .map(|url| codec::redact(url, secret));
    if url.as_deref().unwrap_or("").is_empty() && preview.withheld.is_empty() {
        return None;
    }
    Some(HitThumb {
        url: url.unwrap_or_default(),
        withheld: codec::redact(&preview.withheld, secret),
    })
}

fn parse_hit(value: &Value, secret: Option<&str>) -> Result<SearchHit, Fault> {
    let object = value.as_object().ok_or(Fault::Malformed)?;
    let frame = object.get("frame").cloned().ok_or(Fault::Malformed)?;
    let frame_object = frame.as_object().ok_or(Fault::Malformed)?;
    let captured_at = required(frame_object, "captured_at")?;
    let captured_us = super::mjpeg::captured_us(&captured_at).unwrap_or(0);
    let description = required(object, "generated_description")?;
    let desc_prov = object
        .get("description_provenance")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let retrieval = object
        .get("retrieval")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let score = retrieval
        .get("value")
        .map(|value| match value {
            Value::Number(number) => number.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default();
    let index_state = object
        .get("index")
        .and_then(Value::as_str)
        .unwrap_or("pending")
        .to_owned();
    let (media_state, media_reason) = match object.get("media_availability") {
        Some(Value::String(kind)) => (kind.clone(), String::new()),
        Some(Value::Object(availability)) => {
            if let Some(reason) = availability.get("reason").and_then(Value::as_str) {
                ("unavailable".to_owned(), sanitize_operator_text(reason))
            } else {
                (
                    availability
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("pending")
                        .to_owned(),
                    String::new(),
                )
            }
        }
        _ => ("pending".to_owned(), String::new()),
    };
    let (decisions, authorized_assessments) =
        parse_hit_assessments(object.get("authorized_assessments"), secret);
    Ok(SearchHit {
        frame_id: required(frame_object, "frame_id")?,
        captured_at: codec::redact(&captured_at, secret),
        captured_us,
        description: sanitize_operator_text(&codec::redact(&description, secret)),
        description_provider: sanitize_operator_text(&required(desc_prov, "provider")?),
        description_model: sanitize_operator_text(&required(desc_prov, "resolved_model")?),
        retrieval_score: codec::redact(&score, secret),
        source_id: String::new(),
        source_label: String::new(),
        index_state: sanitize_state(&index_state),
        media_state: sanitize_state(&media_state),
        media_reason,
        command_frame: frame.clone(),
        frame_ref: redact_value(frame, secret),
        thumb: None,
        decisions,
        authorized_assessments,
    })
}

fn parse_hit_assessments(value: Option<&Value>, secret: Option<&str>) -> (Vec<DecisionRow>, bool) {
    let Some(value) = value else {
        return (Vec::new(), false);
    };
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rows = entries
        .iter()
        .filter_map(|entry| parse_decision(entry, secret).ok())
        .collect();
    (rows, true)
}

fn parse_decision(value: &Value, secret: Option<&str>) -> Result<DecisionRow, Fault> {
    let object = value.as_object().ok_or(Fault::Malformed)?;
    let record = object
        .get("record")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let predicate = record
        .get("predicate")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let provenance = record
        .get("provenance")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let producer = provenance.get("producer").and_then(Value::as_object);
    let structured = record
        .get("structured_output")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let probability = structured
        .get("noul_probability")
        .or_else(|| {
            structured
                .get("option_distribution")
                .and_then(Value::as_object)
                .and_then(|dist| dist.get("values"))
                .and_then(Value::as_array)
                .and_then(|values| values.first())
        })
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".to_owned());
    let confidence = structured
        .get("vendor_confidence")
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".to_owned());
    let roi = structured
        .get("evidence_box")
        .and_then(Value::as_object)
        .and_then(|object| object.get("bounds"))
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".to_owned());
    let evidence_state = object
        .get("evidence")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .map(evidence_kind)
        .unwrap_or_else(|| "unknown".to_owned());
    Ok(DecisionRow {
        decision_id: sanitize_operator_text(&required(record, "decision_id")?),
        predicate: format!(
            "{} v{}",
            sanitize_operator_text(&required(predicate, "id")?),
            field_display(predicate.get("version")).unwrap_or_else(|| "?".to_owned())
        ),
        subject: summarize_subject(record.get("subject")),
        outcome: sanitize_state(
            record
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
        ),
        coverage: sanitize_state(
            provenance
                .get("coverage")
                .and_then(Value::as_str)
                .unwrap_or("not_evaluated"),
        ),
        producer: producer
            .and_then(|object| object.get("producer_id"))
            .and_then(Value::as_str)
            .map(sanitize_operator_text)
            .unwrap_or_else(|| "typesafe-jev".to_owned()),
        model_revision: producer
            .and_then(|object| object.get("model_revision"))
            .and_then(Value::as_str)
            .map(sanitize_operator_text)
            .unwrap_or_else(|| "—".to_owned()),
        probability: codec::redact(&probability, secret),
        confidence: codec::redact(&confidence, secret),
        roi: codec::redact(&roi, secret),
        evaluated_at: provenance
            .get("evaluated_at")
            .and_then(Value::as_str)
            .map(|stamp| codec::redact(stamp, secret))
            .unwrap_or_else(|| "—".to_owned()),
        evidence_state: sanitize_state(&evidence_state),
    })
}

fn summarize_subject(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "—".to_owned();
    };
    value
        .get("kind")
        .and_then(Value::as_str)
        .map(sanitize_operator_text)
        .unwrap_or_else(|| "subject".to_owned())
}

fn evidence_kind(value: &Value) -> String {
    match value {
        Value::String(kind) => kind.clone(),
        Value::Object(object) => object
            .get("reason")
            .map(|_| "unavailable".to_owned())
            .unwrap_or_else(|| {
                object
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned()
            }),
        _ => "unknown".to_owned(),
    }
}

fn sanitize_state(value: &str) -> String {
    value.trim().replace('_', "-")
}

fn sanitize_operator_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("://") {
        return "[withheld]".to_owned();
    }
    trimmed.to_owned()
}

fn redact_value(value: Value, secret: Option<&str>) -> Value {
    match value {
        Value::String(text) => Value::String(codec::redact(&text, secret)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| redact_value(item, secret))
                .collect(),
        ),
        Value::Object(object) => {
            let mut next = Map::new();
            for (key, item) in object {
                if key == "store" {
                    continue;
                }
                next.insert(key, redact_value(item, secret));
            }
            Value::Object(next)
        }
        other => other,
    }
}

fn field_display(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_owned()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn required(object: &Map<String, Value>, field: &str) -> Result<String, Fault> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or(Fault::Malformed)
}

fn owned_result<'a>(value: &'a Value, operation: &str) -> Result<&'a Map<String, Value>, Fault> {
    let object = value.as_object().ok_or(Fault::Malformed)?;
    if object.get("operation").and_then(Value::as_str) != Some(operation) {
        return Err(Fault::Malformed);
    }
    object
        .get("result")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)
}

fn decode(reply: &Reply, secret: Option<&str>) -> Result<Outcome, Fault> {
    match codec::decode(reply, secret) {
        Err(ProtocolError::Unauthorized(message)) => Err(Fault::Unauthorized(message)),
        Err(ProtocolError::Forbidden(message)) => Err(Fault::Forbidden(message)),
        Err(ProtocolError::Malformed) => Err(Fault::Malformed),
        Ok(Outcome::Status(status)) => Err(status_fault(&status)),
        Ok(outcome) => Ok(outcome),
    }
}

fn status_fault(status: &codec::RpcStatus) -> Fault {
    let unavailable = status.status == "INTERNAL"
        || status.status == "UNAVAILABLE"
        || status
            .reasons
            .iter()
            .any(|reason| reason == "INTERNAL_ERROR");
    if unavailable {
        Fault::Unavailable(status.message.clone())
    } else {
        Fault::Malformed
    }
}

fn one_payload(outcome: &Outcome) -> Result<String, Fault> {
    let payloads = match outcome {
        Outcome::Exchange(Exchange::Task(task)) => task
            .artifacts
            .iter()
            .flat_map(|artifact| artifact.data.clone())
            .collect::<Vec<_>>(),
        Outcome::Exchange(Exchange::Message(message)) => message.data.clone(),
        Outcome::Tasks(_) | Outcome::Status(_) => Vec::new(),
    };
    let [payload] = payloads.as_slice() else {
        return Err(Fault::Malformed);
    };
    Ok(payload.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Reply;
    use serde_json::json;

    #[test]
    fn capabilities_and_search_roundtrip_without_store_paths() {
        let (provenance, registered) = interpret_semantic_capabilities(
            &reply(json!({
                "operation": "get_semantic_search_capabilities",
                "result": {
                    "registered": true,
                    "active_provenance": {
                        "model_space": {
                            "provider": "local_bge",
                            "model": "BAAI/bge-small-en-v1.5",
                            "dimensions": 384,
                            "normalization": "l2_unit",
                            "metric": "cosine",
                            "generation": 1
                        },
                        "description_generation": 1
                    }
                }
            })),
            None,
        )
        .unwrap();
        assert!(registered);
        let pick = provenance.expect("provenance");
        assert_eq!(
            pick.model_space.get("provider").and_then(Value::as_str),
            Some("local_bge")
        );
        assert_eq!(pick.description_generation, 1);
        assert!(
            interpret_semantic_capabilities(
                &reply(json!({
                    "operation": "get_semantic_search_capabilities",
                    "result": { "registered": true }
                })),
                None,
            )
            .is_err()
        );
        let (float_pick, float_registered) = interpret_semantic_capabilities(
            &reply(json!({
                "operation": "get_semantic_search_capabilities",
                "result": {
                    "registered": true,
                    "active_provenance": {
                        "model_space": {
                            "provider": "bedrock",
                            "model": "text-embed-v1",
                            "dimensions": 4.0,
                            "normalization": "l2_unit",
                            "metric": "cosine",
                            "generation": 1.0
                        },
                        "description_generation": 1.0
                    }
                }
            })),
            None,
        )
        .unwrap();
        assert!(float_registered);
        let float_pick = float_pick.expect("float provenance");
        assert_eq!(float_pick.description_generation, 1);
        assert_eq!(
            float_pick
                .model_space
                .get("dimensions")
                .and_then(Value::as_u64),
            Some(4)
        );
        let (task_pick, task_registered) = interpret_semantic_capabilities(
            &Reply {
                status: 200,
                body: json!({
                    "task": {
                        "id": "task-semantic-cap",
                        "contextId": "ctx-semantic-cap",
                        "status": { "state": "TASK_STATE_COMPLETED" },
                        "artifacts": [{
                            "artifactId": "artifact-0",
                            "parts": [{
                                "data": {
                                    "operation": "get_semantic_search_capabilities",
                                    "result": {
                                        "registered": true,
                                        "active_provenance": {
                                            "description_generation": 1.0,
                                            "model_space": {
                                                "dimensions": 4.0,
                                                "generation": 1.0,
                                                "metric": "cosine",
                                                "model": "text-embed-v1",
                                                "normalization": "l2_unit",
                                                "provider": "bedrock"
                                            }
                                        }
                                    }
                                },
                                "mediaType": "application/json"
                            }]
                        }]
                    }
                })
                .to_string(),
            },
            None,
        )
        .expect("task reply");
        assert!(task_registered);
        assert_eq!(
            task_pick.expect("task provenance").description_generation,
            1
        );
        let (hits, incomplete) = interpret_search(
            &reply(json!({
                "operation": "search_semantic_frames",
                "result": {
                    "incomplete_indexing": true,
                    "hits": { "items": [fixture_hit()] }
                }
            })),
            None,
        )
        .unwrap();
        assert!(incomplete);
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].description.contains('/'));
        assert!(hits[0].frame_ref.get("store").is_none());
        assert_eq!(hits[0].description_model, "global.openai.gpt-5.6-luna");
    }

    #[test]
    fn search_hit_assessments_parse_without_history_request() {
        let mut hit = fixture_hit();
        let frame = hit.get("frame").cloned().unwrap();
        hit.as_object_mut().unwrap().insert(
            "authorized_assessments".to_owned(),
            json!({
                "frame": frame,
                "history_policy": "full_history",
                "entries": [fixture_decision()],
                "fingerprints": { "dec-1": "sha256:0102030405060708090a0b0c0d0e0f0102030405060708090a0b0c0d0e0f0" },
                "truncated": false
            }),
        );
        let (hits, _) = interpret_search(
            &reply(json!({
                "operation": "search_semantic_frames",
                "result": {
                    "incomplete_indexing": false,
                    "hits": { "items": [hit] }
                }
            })),
            None,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].authorized_assessments);
        assert_eq!(hits[0].decisions.len(), 1);
    }

    #[test]
    fn search_hit_empty_assessments_parse() {
        let mut hit = fixture_hit();
        let frame = hit.get("frame").cloned().unwrap();
        hit.as_object_mut().unwrap().insert(
            "authorized_assessments".to_owned(),
            json!({
                "frame": frame,
                "history_policy": "full_history",
                "entries": [],
                "fingerprints": {},
                "truncated": false
            }),
        );
        let (hits, _) = interpret_search(
            &reply(json!({
                "operation": "search_semantic_frames",
                "result": {
                    "incomplete_indexing": false,
                    "hits": { "items": [hit] }
                }
            })),
            None,
        )
        .unwrap();
        assert!(hits[0].authorized_assessments);
        assert!(hits[0].decisions.is_empty());
    }

    #[test]
    fn decision_history_separates_coverage_and_outcome() {
        let rows = interpret_history(
            &reply(json!({
                "operation": "read_frame_decision_history",
                "result": {
                    "entries": { "items": [fixture_decision()] }
                }
            })),
            None,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, "present");
        assert_eq!(rows[0].coverage, "evaluated");
        assert_eq!(rows[0].producer, "typesafe-jev");
    }

    fn reply(body: Value) -> Reply {
        Reply {
            status: 200,
            body: json!({
                "task": {
                    "id": "protocol-task",
                    "contextId": "protocol-context",
                    "status": {"state": "TASK_STATE_COMPLETED"},
                    "artifacts": [{
                        "artifactId": "art-0",
                        "parts": [{"data": body, "mediaType": "application/json"}]
                    }]
                }
            })
            .to_string(),
        }
    }

    fn fixture_hit() -> Value {
        json!({
            "frame": {
                "frame_id": "frame-a",
                "source_id": "cam-a",
                "track_id": "mjpeg-0",
                "captured_at": "2024-01-01T00:00:01Z",
                "payload_digest": "abc",
                "store": "chupacabra.dvr"
            },
            "generated_description": "Pipette tip visible near mount.",
            "description_provenance": {
                "provider": "bedrock",
                "resolved_model": "global.openai.gpt-5.6-luna"
            },
            "retrieval": { "value": 0.91, "metric": "cosine" },
            "index": "ready",
            "media_availability": "ready"
        })
    }

    fn fixture_decision() -> Value {
        json!({
            "record": {
                "decision_id": "dec-1",
                "predicate": { "id": "lab.pipette_tip_attached", "version": 1 },
                "subject": { "kind": "pipette_channel" },
                "outcome": "present",
                "structured_output": {
                    "noul_probability": 0.97,
                    "vendor_confidence": 0.88,
                    "evidence_box": { "bounds": [0.1, 0.2, 0.5, 0.6] }
                },
                "provenance": {
                    "coverage": "evaluated",
                    "evaluated_at": "2024-01-01T00:00:05Z",
                    "producer": { "producer_id": "typesafe-jev", "model_revision": "jev-1.14.0" }
                }
            },
            "evidence": ["ready"]
        })
    }
}
