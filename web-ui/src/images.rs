//! Image history for one source: list, scoped search, and page merges.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::{Map, Value};

use crate::codec::{self, Exchange, Outcome, ProtocolError, Reply};
use crate::inventory::{Fault, Kind};
use crate::metrics::{self, QueryRange};
use crate::workbench::report::{self, ImagePreview};

pub const LIMIT: u32 = 100;
pub const BLANK_SEARCH: &str = "Enter a search, or clear the field.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    pub id: String,
    pub source_id: String,
    pub captured_at: String,
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    pub caption: String,
    pub attributes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<Descriptor>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    List,
    Search {
        text: Option<String>,
        range: Option<(String, String)>,
    },
}

impl Read {
    pub fn is_search(&self) -> bool {
        matches!(self, Self::Search { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourcePhase {
    Loading,
    None,
    Error,
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub generation: u32,
    pub source_id: String,
    pub sources: Vec<String>,
    pub read: Read,
    pub cursor: Option<String>,
    pub append: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reader {
    pub generation: u32,
    pub source_id: String,
    pub all: bool,
    pub read: Read,
    pub items: Vec<Descriptor>,
    pub selected: String,
    pub next_cursor: Option<String>,
    pub loading: bool,
    pub fault: String,
    pub validation: String,
    expected: Option<Ticket>,
    settled: bool,
    catalog: Vec<String>,
}

impl Reader {
    pub fn new() -> Self {
        Self {
            generation: 0,
            source_id: String::new(),
            all: false,
            read: Read::List,
            items: Vec::new(),
            selected: String::new(),
            next_cursor: None,
            loading: false,
            fault: String::new(),
            validation: String::new(),
            expected: None,
            settled: false,
            catalog: Vec::new(),
        }
    }

    pub fn note_catalog(&mut self, phase: SourcePhase, ids: &[String]) -> Option<Ticket> {
        match phase {
            SourcePhase::Loading => None,
            SourcePhase::Error | SourcePhase::None => {
                self.clear_history();
                None
            }
            SourcePhase::Ready => self.note_sources(ids),
        }
    }

    pub fn select_source(&mut self, id: &str, ids: &[String]) -> Option<Ticket> {
        if !ids.iter().any(|item| item == id) || (!self.all && id == self.source_id) {
            return None;
        }
        self.all = false;
        if !self.validation.is_empty() {
            self.clear_history();
            self.source_id = id.to_owned();
            return None;
        }
        let read = self.read.clone();
        Some(self.issue(id.to_owned(), read, None, false))
    }

    pub fn select_all(&mut self) -> Option<Ticket> {
        if self.all || self.catalog.is_empty() {
            return None;
        }
        self.all = true;
        if !self.validation.is_empty() {
            self.clear_history();
            self.all = true;
            return None;
        }
        let read = self.read.clone();
        Some(self.issue(String::new(), read, None, false))
    }

    pub fn commit(
        &mut self,
        text: &str,
        start: &str,
        end: &str,
    ) -> Result<Option<Ticket>, &'static str> {
        let read = match plan(text, start, end) {
            Ok(read) => read,
            Err(message) => {
                self.validation = message.to_owned();
                return Err(message);
            }
        };
        self.validation.clear();
        if read == self.read && self.fault.is_empty() && (self.settled || self.loading) {
            return Ok(None);
        }
        if self.all {
            return Ok(Some(self.issue(String::new(), read, None, false)));
        }
        if self.source_id.is_empty() {
            self.read = read;
            return Ok(None);
        }
        let source = self.source_id.clone();
        Ok(Some(self.issue(source, read, None, false)))
    }

    pub fn more(&mut self) -> Option<Ticket> {
        if self.loading || (!self.all && self.source_id.is_empty()) || !self.validation.is_empty() {
            return None;
        }
        let cursor = self
            .next_cursor
            .clone()
            .filter(|cursor| !cursor.is_empty())?;
        let source = if self.all {
            String::new()
        } else {
            self.source_id.clone()
        };
        let read = self.read.clone();
        Some(self.issue(source, read, Some(cursor), true))
    }

    pub fn select_result(&mut self, id: &str) {
        self.selected = keep_result(id, &self.items);
    }

    pub fn accept(&mut self, ticket: &Ticket, page: Page) -> bool {
        if self.expected.as_ref() != Some(ticket) {
            return false;
        }
        self.expected = None;
        self.loading = false;
        self.fault.clear();
        self.settled = true;
        self.items = if ticket.append {
            merge(&self.items, &page.items)
        } else {
            page.items
        };
        self.selected = keep_result(&self.selected, &self.items);
        self.next_cursor = page.next_cursor;
        true
    }

    pub fn reject(&mut self, ticket: &Ticket, message: &str) -> bool {
        if self.expected.as_ref() != Some(ticket) {
            return false;
        }
        self.expected = None;
        self.loading = false;
        self.fault = message.to_owned();
        self.settled = false;
        true
    }

    pub fn cancel(&mut self, ticket: &Ticket) -> bool {
        if self.expected.as_ref() != Some(ticket) {
            return false;
        }
        self.expected = None;
        self.loading = false;
        true
    }

    fn note_sources(&mut self, ids: &[String]) -> Option<Ticket> {
        let catalog_same = self.catalog == ids;
        self.catalog = ids.to_vec();
        if self.all {
            if ids.is_empty() {
                self.clear_history();
                return None;
            }
            if catalog_same && self.fault.is_empty() && (self.loading || self.settled) {
                return None;
            }
            if catalog_same && !self.fault.is_empty() {
                return None;
            }
            if !self.validation.is_empty() {
                self.clear_history();
                self.all = true;
                return None;
            }
            let read = self.read.clone();
            return Some(self.issue(String::new(), read, None, false));
        }
        let next = keep_source(&self.source_id, ids);
        if next.is_empty() {
            self.clear_history();
            return None;
        }
        let same = next == self.source_id;
        if same && self.fault.is_empty() && (self.loading || self.settled) {
            return None;
        }
        if same && !self.fault.is_empty() {
            return None;
        }
        if !self.validation.is_empty() {
            self.clear_history();
            self.source_id = next;
            return None;
        }
        let read = self.read.clone();
        Some(self.issue(next, read, None, false))
    }

    fn clear_history(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.expected = None;
        self.loading = false;
        self.all = false;
        self.source_id.clear();
        self.items.clear();
        self.selected.clear();
        self.next_cursor = None;
        self.fault.clear();
        self.settled = false;
    }

    fn issue(
        &mut self,
        source_id: String,
        read: Read,
        cursor: Option<String>,
        append: bool,
    ) -> Ticket {
        self.generation = self.generation.wrapping_add(1);
        self.source_id = source_id;
        self.read = read;
        self.loading = true;
        self.fault.clear();
        if !append {
            self.settled = false;
        }
        let ticket = Ticket {
            generation: self.generation,
            source_id: self.source_id.clone(),
            sources: if self.source_id.is_empty() {
                self.catalog.clone()
            } else {
                Vec::new()
            },
            read: self.read.clone(),
            cursor,
            append,
        };
        self.expected = Some(ticket.clone());
        ticket
    }
}

pub fn source_phase(kind: Kind, count: usize) -> SourcePhase {
    match kind {
        Kind::Empty => SourcePhase::None,
        Kind::Populated if count == 0 => SourcePhase::None,
        Kind::Populated => SourcePhase::Ready,
        Kind::Unavailable
        | Kind::Unauthorized
        | Kind::Forbidden
        | Kind::Malformed
        | Kind::Transport => SourcePhase::Error,
        Kind::Pending | Kind::Advertised | Kind::Unadvertised | Kind::Loading => {
            SourcePhase::Loading
        }
    }
}

pub fn source_copy(phase: SourcePhase) -> &'static str {
    match phase {
        SourcePhase::Loading => "Loading image sources.",
        SourcePhase::None => "No image sources.",
        SourcePhase::Error => "Image sources could not be read.",
        SourcePhase::Ready => "",
    }
}

pub fn source_placeholder(phase: SourcePhase) -> &'static str {
    if phase == SourcePhase::Loading {
        "Loading"
    } else {
        "None loaded"
    }
}

pub fn history_status(reader: &Reader, source_ready: bool) -> (&'static str, &'static str) {
    if !source_ready
        || (!reader.validation.is_empty() && !reader.settled && reader.items.is_empty())
    {
        return ("idle", "");
    }
    if reader.loading {
        return ("loading", "Loading images.");
    }
    if !reader.fault.is_empty() {
        return ("error", "Images could not be read.");
    }
    if reader.items.is_empty() {
        if reader.read.is_search() {
            return ("nomatch", "No images match this search.");
        }
        return ("empty", "No images for this source.");
    }
    ("ready", "")
}

pub fn history_alert(reader: &Reader) -> &str {
    if !reader.validation.is_empty() {
        return reader.validation.as_str();
    }
    reader.fault.as_str()
}

pub fn plan(text: &str, start: &str, end: &str) -> Result<Read, &'static str> {
    let range = metrics::query_range(start, end)?;
    let caption = text.trim();
    if caption.is_empty() && !text.is_empty() && matches!(range, QueryRange::Window) {
        return Err(BLANK_SEARCH);
    }
    let text = (!caption.is_empty()).then(|| caption.to_owned());
    match range {
        QueryRange::Window if text.is_none() => Ok(Read::List),
        QueryRange::Window => Ok(Read::Search { text, range: None }),
        QueryRange::Bounded { start, end } => Ok(Read::Search {
            text,
            range: Some((start, end)),
        }),
    }
}

pub fn command(source_id: &str, read: &Read, cursor: Option<&str>) -> Value {
    let mut page = Map::new();
    page.insert("limit".to_owned(), Value::from(LIMIT));
    if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
        page.insert("cursor".to_owned(), Value::String(cursor.to_owned()));
    }
    match read {
        Read::List => serde_json::json!({
            "operation": "list_images",
            "params": { "source_id": source_id, "page": page }
        }),
        Read::Search { text, range } => {
            let mut params = Map::new();
            if !source_id.is_empty() {
                params.insert("source_id".to_owned(), Value::String(source_id.to_owned()));
            }
            if let Some(text) = text {
                params.insert("text".to_owned(), Value::String(text.clone()));
            }
            if let Some((start, end)) = range {
                params.insert(
                    "range".to_owned(),
                    serde_json::json!({ "start": start, "end": end }),
                );
            }
            params.insert("page".to_owned(), Value::Object(page));
            serde_json::json!({ "operation": "search_images", "params": params })
        }
    }
}

pub fn merge(existing: &[Descriptor], incoming: &[Descriptor]) -> Vec<Descriptor> {
    let mut merged = existing.to_vec();
    for item in incoming {
        if merged.iter().any(|have| have.id == item.id) {
            continue;
        }
        merged.push(item.clone());
    }
    merged
}

pub fn keep_source(selected: &str, sources: &[String]) -> String {
    if sources.iter().any(|id| id == selected) {
        selected.to_owned()
    } else {
        sources.first().cloned().unwrap_or_default()
    }
}

pub fn keep_result(selected: &str, items: &[Descriptor]) -> String {
    if items.iter().any(|item| item.id == selected) {
        selected.to_owned()
    } else {
        String::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectTicket {
    pub generation: u32,
    pub source_id: String,
    pub image_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub id: String,
    pub source_id: String,
    pub captured_at: String,
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    pub caption: String,
    pub attributes: String,
    pub preview: ImagePreview,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspector {
    generation: u32,
    expected: Option<InspectTicket>,
    bound: String,
    image_id: String,
    pub loading: bool,
    pub frame: Option<Frame>,
    pub fault: String,
}

impl Inspector {
    pub fn new() -> Self {
        Self {
            generation: 0,
            expected: None,
            bound: String::new(),
            image_id: String::new(),
            loading: false,
            frame: None,
            fault: String::new(),
        }
    }

    pub fn rebind(&mut self, source_id: &str) {
        if source_id == self.bound {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.expected = None;
        self.bound = source_id.to_owned();
        self.image_id.clear();
        self.loading = false;
        self.frame = None;
        self.fault.clear();
    }

    pub fn open_selected(&mut self, source_id: &str, image_id: &str) -> Option<InspectTicket> {
        if source_id.is_empty()
            || image_id.is_empty()
            || (!self.bound.is_empty() && source_id != self.bound)
        {
            return None;
        }
        Some(self.issue(source_id, image_id))
    }

    pub fn accept(&mut self, ticket: &InspectTicket, frame: Frame) -> bool {
        if self.expected.as_ref() != Some(ticket) {
            return false;
        }
        self.expected = None;
        self.loading = false;
        if !self.bound.is_empty() && ticket.source_id != self.bound {
            return false;
        }
        self.fault.clear();
        self.frame = Some(frame);
        true
    }

    pub fn reject(&mut self, ticket: &InspectTicket, message: &str) -> bool {
        if self.expected.as_ref() != Some(ticket) {
            return false;
        }
        self.expected = None;
        self.loading = false;
        self.frame = None;
        self.fault = message.to_owned();
        true
    }

    pub fn cancel(&mut self, ticket: &InspectTicket) -> bool {
        if self.expected.as_ref() != Some(ticket) {
            return false;
        }
        self.expected = None;
        self.loading = false;
        true
    }

    fn issue(&mut self, source_id: &str, image_id: &str) -> InspectTicket {
        self.generation = self.generation.wrapping_add(1);
        self.image_id = image_id.to_owned();
        self.loading = true;
        self.fault.clear();
        self.frame = None;
        let ticket = InspectTicket {
            generation: self.generation,
            source_id: source_id.to_owned(),
            image_id: image_id.to_owned(),
        };
        self.expected = Some(ticket.clone());
        ticket
    }
}

pub fn inspect_status(inspector: &Inspector) -> (&'static str, &'static str) {
    if inspector.loading {
        return ("selected", "Loading the selected image.");
    }
    if !inspector.fault.is_empty() {
        return ("selected-error", "The selected image could not be read.");
    }
    if inspector.frame.is_some() {
        return ("selected-ready", "");
    }
    ("idle", "")
}

pub fn preview_alt(frame: &Frame) -> String {
    let caption = frame.caption.trim();
    if !caption.is_empty() {
        return caption.to_owned();
    }
    if frame.id.is_empty() {
        return "Image".to_owned();
    }
    if frame.source_id.is_empty() {
        return frame.id.clone();
    }
    format!("{} from {}", frame.id, frame.source_id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumb {
    pub url: String,
    pub withheld: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbTicket {
    pub generation: u32,
    pub source_id: String,
    pub image_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gallery {
    generation: u32,
    source_id: String,
    thumbs: std::collections::BTreeMap<String, Thumb>,
    queue: Vec<String>,
    inflight: Option<ThumbTicket>,
}

impl Gallery {
    pub fn new() -> Self {
        Self {
            generation: 0,
            source_id: String::new(),
            thumbs: std::collections::BTreeMap::new(),
            queue: Vec::new(),
            inflight: None,
        }
    }

    pub fn retarget(&mut self, source_id: &str) {
        if source_id == self.source_id {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.source_id = source_id.to_owned();
        self.thumbs.clear();
        self.queue.clear();
        self.inflight = None;
    }

    pub fn get(&self, id: &str) -> Option<&Thumb> {
        self.thumbs.get(id)
    }

    pub fn note(&mut self, source_id: &str, ids: &[String]) -> Option<ThumbTicket> {
        self.retarget(source_id);
        for id in ids {
            self.enqueue(id);
        }
        self.dispatch()
    }

    pub fn accept(&mut self, ticket: &ThumbTicket, thumb: Thumb) -> bool {
        if !self.matches(ticket) {
            return false;
        }
        self.inflight = None;
        self.thumbs.insert(ticket.image_id.clone(), thumb);
        true
    }

    pub fn reject(&mut self, ticket: &ThumbTicket) -> bool {
        if !self.matches(ticket) {
            return false;
        }
        self.inflight = None;
        self.thumbs.insert(
            ticket.image_id.clone(),
            Thumb {
                url: String::new(),
                withheld: "Preview unavailable.".to_owned(),
            },
        );
        true
    }

    pub fn pump(&mut self) -> Option<ThumbTicket> {
        self.dispatch()
    }

    fn enqueue(&mut self, id: &str) {
        if id.is_empty() || self.thumbs.contains_key(id) {
            return;
        }
        if self
            .inflight
            .as_ref()
            .is_some_and(|ticket| ticket.image_id == id)
        {
            return;
        }
        if self.queue.iter().any(|queued| queued == id) {
            return;
        }
        self.queue.push(id.to_owned());
    }

    fn dispatch(&mut self) -> Option<ThumbTicket> {
        if self.inflight.is_some() || self.source_id.is_empty() {
            return None;
        }
        let image_id = self.queue.first().cloned()?;
        self.queue.remove(0);
        let ticket = ThumbTicket {
            generation: self.generation,
            source_id: self.source_id.clone(),
            image_id,
        };
        self.inflight = Some(ticket.clone());
        Some(ticket)
    }

    fn matches(&self, ticket: &ThumbTicket) -> bool {
        self.inflight.as_ref() == Some(ticket) && ticket.generation == self.generation
    }
}

pub fn thumb_from(frame: &Frame) -> Thumb {
    Thumb {
        url: frame.preview.preview_url.clone().unwrap_or_default(),
        withheld: frame.preview.withheld.clone(),
    }
}

pub fn image_command(image_id: &str) -> Value {
    serde_json::json!({
        "operation": "get_image",
        "params": { "id": image_id }
    })
}

pub fn interpret_frame(reply: &Reply, secret: Option<&str>) -> Result<Frame, Fault> {
    let outcome = match codec::decode(reply, secret) {
        Err(ProtocolError::Unauthorized(message)) => return Err(Fault::Unauthorized(message)),
        Err(ProtocolError::Forbidden(message)) => return Err(Fault::Forbidden(message)),
        Err(ProtocolError::Malformed) => return Err(Fault::Malformed),
        Ok(Outcome::Status(status)) => return Err(status_fault(&status)),
        Ok(outcome) => outcome,
    };
    let mut frame = frame_from(&outcome)?;
    redact_frame(&mut frame, secret);
    Ok(frame)
}

pub fn interpret(reply: &Reply, secret: Option<&str>) -> Result<Page, Fault> {
    let mut page = match codec::decode(reply, secret) {
        Err(ProtocolError::Unauthorized(message)) => return Err(Fault::Unauthorized(message)),
        Err(ProtocolError::Forbidden(message)) => return Err(Fault::Forbidden(message)),
        Err(ProtocolError::Malformed) => return Err(Fault::Malformed),
        Ok(Outcome::Status(status)) => return Err(status_fault(&status)),
        Ok(outcome) => page_from(&outcome)?,
    };
    for item in &mut page.items {
        item.id = codec::redact(&item.id, secret);
        item.source_id = codec::redact(&item.source_id, secret);
        item.captured_at = codec::redact(&item.captured_at, secret);
        item.media_type = codec::redact(&item.media_type, secret);
        item.caption = codec::redact(&item.caption, secret);
        item.attributes = codec::redact(&item.attributes, secret);
    }
    Ok(page)
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

fn frame_from(outcome: &Outcome) -> Result<Frame, Fault> {
    let payloads = exchange_payloads(outcome);
    let [payload] = payloads.as_slice() else {
        return Err(Fault::Malformed);
    };
    parse_frame(payload)
}

fn parse_frame(payload: &str) -> Result<Frame, Fault> {
    let value: Value = serde_json::from_str(payload).map_err(|_| Fault::Malformed)?;
    let object = value.as_object().ok_or(Fault::Malformed)?;
    let expected = "get_image";
    if object.get("operation").and_then(Value::as_str) != Some(expected) {
        return Err(Fault::Malformed);
    }
    let result = object
        .get("result")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let descriptor = parse_descriptor(result.get("descriptor").ok_or(Fault::Malformed)?)?;
    let encoded = result
        .get("data")
        .and_then(Value::as_str)
        .ok_or(Fault::Malformed)?;
    let preview = report::preview_for(&descriptor.media_type, encoded);
    Ok(Frame {
        id: descriptor.id,
        source_id: descriptor.source_id,
        captured_at: descriptor.captured_at,
        media_type: descriptor.media_type,
        width: descriptor.width,
        height: descriptor.height,
        caption: descriptor.caption,
        attributes: descriptor.attributes,
        preview,
    })
}

fn redact_frame(frame: &mut Frame, secret: Option<&str>) {
    frame.id = codec::redact(&frame.id, secret);
    frame.source_id = codec::redact(&frame.source_id, secret);
    frame.captured_at = codec::redact(&frame.captured_at, secret);
    frame.media_type = codec::redact(&frame.media_type, secret);
    frame.caption = codec::redact(&frame.caption, secret);
    frame.attributes = codec::redact(&frame.attributes, secret);
    frame.preview.media_type = codec::redact(&frame.preview.media_type, secret);
    frame.preview.withheld = codec::redact(&frame.preview.withheld, secret);
    frame.preview.preview_url = frame
        .preview
        .preview_url
        .as_ref()
        .map(|url| codec::redact(url, secret));
}

fn page_from(outcome: &Outcome) -> Result<Page, Fault> {
    let payloads = exchange_payloads(outcome);
    if payloads.is_empty() {
        return Err(Fault::Malformed);
    }
    let mut items = Vec::new();
    let mut next_cursor = None;
    for payload in payloads {
        let page = parse_payload(&payload)?;
        next_cursor = page.next_cursor;
        items.extend(page.items);
    }
    Ok(Page { items, next_cursor })
}

fn exchange_payloads(outcome: &Outcome) -> Vec<String> {
    match outcome {
        Outcome::Exchange(Exchange::Task(task)) => task
            .artifacts
            .iter()
            .flat_map(|artifact| artifact.data.clone())
            .collect(),
        Outcome::Exchange(Exchange::Message(message)) => message.data.clone(),
        Outcome::Tasks(_) | Outcome::Status(_) => Vec::new(),
    }
}

fn parse_payload(payload: &str) -> Result<Page, Fault> {
    let value: Value = serde_json::from_str(payload).map_err(|_| Fault::Malformed)?;
    let object = value.as_object().ok_or(Fault::Malformed)?;
    let operation = object.get("operation").and_then(Value::as_str);
    if operation != Some("list_images") && operation != Some("search_images") {
        return Err(Fault::Malformed);
    }
    let result = object
        .get("result")
        .and_then(Value::as_object)
        .ok_or(Fault::Malformed)?;
    let raw = result
        .get("items")
        .and_then(Value::as_array)
        .ok_or(Fault::Malformed)?;
    let mut items = Vec::new();
    for item in raw {
        items.push(parse_descriptor(item)?);
    }
    Ok(Page {
        items,
        next_cursor: cursor(result.get("next_cursor"))?,
    })
}

fn parse_descriptor(value: &Value) -> Result<Descriptor, Fault> {
    let object = value.as_object().ok_or(Fault::Malformed)?;
    Ok(Descriptor {
        id: required(object, "id")?,
        source_id: required(object, "source_id")?,
        captured_at: required(object, "captured_at")?,
        media_type: required(object, "media_type")?,
        width: dimension(object, "width")?,
        height: dimension(object, "height")?,
        caption: caption(object.get("caption"))?,
        attributes: attributes(object.get("attributes"))?,
    })
}

fn required(object: &Map<String, Value>, key: &str) -> Result<String, Fault> {
    match object.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_owned()),
        _ => Err(Fault::Malformed),
    }
}

fn dimension(object: &Map<String, Value>, key: &str) -> Result<u32, Fault> {
    let Some(value) = object.get(key) else {
        return Err(Fault::Malformed);
    };
    let number = value.as_u64().or_else(|| whole_number(value));
    let Some(number) = number else {
        return Err(Fault::Malformed);
    };
    u32::try_from(number).map_err(|_| Fault::Malformed)
}

fn whole_number(value: &Value) -> Option<u64> {
    let number = value.as_f64()?;
    if !number.is_finite() || number < 0.0 || number.fract() != 0.0 {
        return None;
    }
    Some(number as u64)
}

fn caption(value: Option<&Value>) -> Result<String, Fault> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(Fault::Malformed),
    }
}

fn attributes(value: Option<&Value>) -> Result<String, Fault> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::Object(fields)) if fields.is_empty() => Ok(String::new()),
        Some(value) => serde_json::to_string(value).map_err(|_| Fault::Malformed),
    }
}

fn cursor(value: Option<&Value>) -> Result<Option<String>, Fault> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(cursor)) if !cursor.is_empty() => Ok(Some(cursor.clone())),
        Some(_) => Err(Fault::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queries_list_until_a_caption_or_range_is_present() {
        assert_eq!(plan("", "", "").unwrap(), Read::List);
        let listed = command("cam-a", &Read::List, None);
        assert_eq!(listed["operation"], "list_images");
        assert_eq!(listed["params"]["source_id"], "cam-a");
        assert!(listed["params"].get("text").is_none());
        assert!(listed["params"].get("range").is_none());
        assert!(listed["params"]["page"].get("cursor").is_none());
        assert_eq!(listed["params"]["page"]["limit"], LIMIT);

        let caption = plan("  plate  ", "", "").unwrap();
        let searched = command("cam-a", &caption, Some("2"));
        assert_eq!(searched["operation"], "search_images");
        assert_eq!(searched["params"]["source_id"], "cam-a");
        assert_eq!(searched["params"]["text"], "plate");
        assert!(searched["params"].get("range").is_none());
        assert_eq!(searched["params"]["page"]["cursor"], "2");
        let every = command("", &caption, None);
        assert!(every["params"].get("source_id").is_none());

        let ranged = plan("   ", "2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z").unwrap();
        let body = command("cam-a", &ranged, None);
        assert!(body["params"].get("text").is_none());
        assert_eq!(
            body["params"]["range"]["start"],
            "2024-01-01T00:00:00.000000000Z"
        );
        assert_eq!(
            body["params"]["range"]["end"],
            "2024-01-01T01:00:00.000000000Z"
        );
        assert_eq!(body["params"]["source_id"], "cam-a");
    }

    #[test]
    fn partial_blank_and_reversed_criteria_do_not_build_a_search() {
        assert_eq!(plan("   ", "", "").unwrap_err(), BLANK_SEARCH);
        assert_eq!(
            plan("", "2024-01-02T00:00:00Z", "").unwrap_err(),
            "Enter a start and an end."
        );
        assert_eq!(
            plan("plate", "2024-01-02T00:00:00Z", "2024-01-01T00:00:00Z").unwrap_err(),
            "The start must be before the end."
        );
        assert_eq!(
            plan("", "2024-01-01T00:00:00+01:00", "2024-01-02T00:00:00Z").unwrap_err(),
            "Enter a UTC time range."
        );
    }

    #[test]
    fn pages_keep_server_order_drop_duplicates_and_keep_selection() {
        let first = desc("a");
        let second = desc("b");
        let third = desc("c");
        let merged = merge(
            &[first.clone(), second.clone()],
            &[second.clone(), third.clone()],
        );
        assert_eq!(
            merged
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_eq!(keep_result("b", &merged), "b");
        assert_eq!(keep_result("missing", &merged), "");
        assert_eq!(keep_source("cam-b", &ids("cam-a", "cam-b")), "cam-b");
        assert_eq!(keep_source("gone", &ids("cam-a", "cam-b")), "cam-a");
        assert_eq!(keep_source("", &ids("cam-b", "cam-a")), "cam-b");

        let mut reader = Reader::new();
        let sources = ids("cam-b", "cam-a");
        let ticket = reader.note_catalog(SourcePhase::Ready, &sources).unwrap();
        assert_eq!(ticket.source_id, "cam-b");
        assert!(reader.accept(
            &ticket,
            page(vec![first.clone(), second.clone()], Some("2"))
        ));
        reader.select_result("b");
        assert!(reader.note_catalog(SourcePhase::Ready, &sources).is_none());
        assert_eq!(reader.source_id, "cam-b");
        assert_eq!(reader.selected, "b");
        let search = reader.commit("b", "", "").unwrap().unwrap();
        assert!(reader.accept(&search, page(vec![first.clone(), second.clone()], None)));
        assert_eq!(reader.selected, "b");
        let listed = reader.commit("", "", "").unwrap().unwrap();
        assert!(reader.accept(&listed, page(vec![first], None)));
        assert_eq!(reader.selected, "");
        let only = vec!["cam-a".to_owned()];
        let moved = reader.note_catalog(SourcePhase::Ready, &only).unwrap();
        assert_eq!(moved.source_id, "cam-a");
        assert!(!reader.all);

        let mut wide = Reader::new();
        let catalog = ids("cam-a", "cam-b");
        let opened = wide.note_catalog(SourcePhase::Ready, &catalog).unwrap();
        assert_eq!(opened.source_id, "cam-a");
        assert!(!wide.all);
        let all = wide.select_all().unwrap();
        assert!(all.source_id.is_empty());
        assert_eq!(all.sources, catalog);
        assert!(wide.all);
        assert!(wide.select_all().is_none());
        let back = wide.select_source("cam-b", &catalog).unwrap();
        assert_eq!(back.source_id, "cam-b");
        assert!(back.sources.is_empty());
        assert!(!wide.all);
    }

    #[test]
    fn stale_source_filter_and_page_outcomes_do_not_replace_the_view() {
        let mut reader = Reader::new();
        let sources = ids("cam-a", "cam-b");
        let listed = reader.note_catalog(SourcePhase::Ready, &sources).unwrap();
        assert!(reader.commit("", "2024-01-02T00:00:00Z", "").is_err());
        assert!(reader.accept(&listed, page(vec![desc("listed")], Some("2"))));
        assert_eq!(reader.items[0].id, "listed");
        assert_eq!(reader.validation, "Enter a start and an end.");

        let search = reader.commit("plate", "", "").unwrap().unwrap();
        assert!(!reader.accept(&listed, page(vec![desc("stale-list")], None)));
        assert!(reader.loading);
        assert!(reader.accept(&search, page(vec![desc("hit")], None)));
        assert_eq!(reader.items[0].id, "hit");

        let again = reader.commit("", "", "").unwrap().unwrap();
        assert!(reader.accept(&again, page(vec![desc("listed")], Some("2"))));
        let more = reader.more().unwrap();
        let fresh = reader.commit("next", "", "").unwrap().unwrap();
        assert!(!reader.accept(&more, page(vec![desc("page-2")], None)));
        assert_eq!(reader.items[0].id, "listed");
        assert!(reader.accept(&fresh, page(vec![desc("fresh")], None)));
        assert_eq!(reader.items[0].id, "fresh");

        let current = reader.commit("", "", "").unwrap().unwrap();
        let switched = reader.select_source("cam-b", &sources).unwrap();
        assert!(!reader.reject(&current, "late"));
        assert!(reader.fault.is_empty());
        assert!(reader.accept(&switched, page(vec![desc("other")], None)));
        assert_eq!(reader.source_id, "cam-b");
        assert_eq!(reader.items[0].id, "other");
    }

    #[test]
    fn descriptors_keep_order_caption_and_attributes() {
        let reply = page_reply(json!({
            "operation": "list_images",
            "result": {
                "items": [
                    {
                        "id": "frame-a",
                        "source_id": "cam-a",
                        "captured_at": "2024-01-01T00:00:00Z",
                        "media_type": "image/png",
                        "width": 8.0,
                        "height": 4.0,
                        "caption": "plate well",
                        "attributes": {"slot": "1"}
                    },
                    {
                        "id": "frame-b",
                        "source_id": "cam-a",
                        "captured_at": "2024-01-01T00:01:00Z",
                        "media_type": "image/jpeg",
                        "width": 2,
                        "height": 2,
                        "attributes": {}
                    }
                ],
                "next_cursor": "2"
            }
        }));
        let page = interpret(&reply, Some("plate well")).unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("2"));
        assert_eq!(page.items[0].id, "frame-a");
        assert_eq!(page.items[0].caption, "[redacted]");
        assert_eq!(page.items[0].width, 8);
        assert_eq!(page.items[0].height, 4);
        assert_eq!(page.items[0].attributes, r#"{"slot":"1"}"#);
        assert_eq!(page.items[1].caption, "");
        assert_eq!(page.items[1].attributes, "");
        assert_eq!(page.items[1].media_type, "image/jpeg");
        assert_eq!(history_status(&Reader::new(), false).0, "idle");
        assert_eq!(source_copy(SourcePhase::None), "No image sources.");
        assert_eq!(
            source_copy(SourcePhase::Error),
            "Image sources could not be read."
        );
        assert_eq!(source_placeholder(SourcePhase::Loading), "Loading");
        assert_eq!(source_placeholder(SourcePhase::None), "None loaded");
        assert_eq!(source_placeholder(SourcePhase::Error), "None loaded");
    }

    fn ids(first: &str, second: &str) -> Vec<String> {
        vec![first.to_owned(), second.to_owned()]
    }

    fn desc(id: &str) -> Descriptor {
        Descriptor {
            id: id.to_owned(),
            source_id: "cam-a".to_owned(),
            captured_at: "2024-01-01T00:00:00Z".to_owned(),
            media_type: "image/png".to_owned(),
            width: 1,
            height: 1,
            caption: String::new(),
            attributes: String::new(),
        }
    }

    fn page(items: Vec<Descriptor>, next_cursor: Option<&str>) -> Page {
        Page {
            items,
            next_cursor: next_cursor.map(str::to_owned),
        }
    }

    #[test]
    fn preview_decisions_follow_the_raster_policy() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let markup = r#"<svg id="image-xss" onload="window.__imageXss=1"></svg>"#;
        for media in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            let frame = shown(media, "AAAA", "frame-a", "plate");
            let preview = frame.preview;
            assert_eq!(
                preview.preview_url.as_deref(),
                Some(format!("data:{media};base64,AAAA")).as_deref()
            );
            assert!(preview.withheld.is_empty());
            assert_eq!(preview.decoded_bytes, Some(3));
            assert!(!preview.preview_url.unwrap_or_default().contains('<'));
        }
        let typed = shown("image/png; charset=binary", png, "frame-a", "plate well");
        assert!(
            typed
                .preview
                .preview_url
                .as_deref()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(preview_alt(&typed), "plate well");
        assert!(typed.preview.decoded_bytes.is_some());

        let svg = shown("image/svg+xml", png, "frame-svg", markup);
        assert!(svg.preview.preview_url.is_none());
        assert!(svg.preview.withheld.contains("allowlist"));
        assert_eq!(svg.caption, markup);
        assert!(!format!("{:?}", svg.preview).contains("onload"));
        assert_eq!(preview_alt(&svg), markup);

        let malformed = shown("image/gif", "****", "frame-bad", "");
        assert!(malformed.preview.preview_url.is_none());
        assert!(malformed.preview.withheld.contains("base64"));
        assert!(malformed.preview.decoded_bytes.is_none());
        assert_eq!(preview_alt(&malformed), "frame-bad from cam-a");

        let huge = "A".repeat((crate::workbench::BROWSER_PREVIEW_MAX_BYTES + 3) / 3 * 4);
        let oversized = shown("image/webp", &huge, "frame-big", "");
        assert!(
            oversized.preview.decoded_bytes.unwrap() > crate::workbench::BROWSER_PREVIEW_MAX_BYTES
        );
        assert!(oversized.preview.preview_url.is_none());
        assert!(oversized.preview.withheld.contains("262144"));
        assert!(!oversized.preview.withheld.contains(&huge));

        let secret = shown_secret("image/png", "AAAA", "token-value");
        assert_eq!(secret.caption, "[redacted]");
        assert!(
            secret
                .preview
                .preview_url
                .unwrap()
                .starts_with("data:image/png;base64,AAAA")
        );
    }

    #[test]
    fn stale_image_retrieval_does_not_replace_the_inspector_or_history() {
        let mut reader = Reader::new();
        let sources = ids("cam-a", "cam-b");
        let listed = reader.note_catalog(SourcePhase::Ready, &sources).unwrap();
        assert!(reader.accept(&listed, page(vec![desc("frame-a"), desc("frame-b")], None)));
        reader.select_result("frame-a");
        let history = reader.clone();

        let mut inspector = Inspector::new();
        inspector.rebind("cam-a");
        let first = inspector.open_selected("cam-a", "frame-a").unwrap();
        let second = inspector.open_selected("cam-a", "frame-b").unwrap();
        assert_eq!(
            inspect_status(&inspector),
            ("selected", "Loading the selected image.")
        );
        assert!(!inspector.accept(&first, sample_frame("frame-a")));
        assert!(inspector.frame.is_none());
        assert!(inspector.accept(&second, sample_frame("frame-b")));
        assert_eq!(inspector.frame.as_ref().unwrap().id, "frame-b");

        let selected = inspector.open_selected("cam-a", "frame-a").unwrap();
        assert!(!inspector.reject(&second, "late"));
        assert!(inspector.fault.is_empty());
        assert!(inspector.loading);
        inspector.rebind("cam-b");
        assert!(!inspector.accept(&selected, sample_frame("frame-a")));
        assert!(inspector.frame.is_none());
        assert_eq!(reader.items.len(), history.items.len());
        assert_eq!(reader.next_cursor, history.next_cursor);
        assert_eq!(reader.read, history.read);

        inspector.rebind("cam-a");
        let failed = inspector.open_selected("cam-a", "frame-a").unwrap();
        assert!(inspector.reject(&failed, "selected offline"));
        assert_eq!(
            inspect_status(&inspector).1,
            "The selected image could not be read."
        );
        assert_eq!(inspector.fault, "selected offline");
        assert_ne!(
            history_status(&reader, true).1,
            inspect_status(&inspector).1
        );
        let retry = inspector.open_selected("cam-a", "frame-a").unwrap();
        assert_eq!(retry.image_id, "frame-a");
        assert!(inspector.reject(&retry, "still offline"));
        assert_eq!(reader, history);
    }

    #[test]
    fn gallery_loads_each_id_once_and_drops_stale_thumbnails() {
        let mut gallery = Gallery::new();
        let first = gallery
            .note("cam-a", &["frame-a".to_owned(), "frame-b".to_owned()])
            .unwrap();
        assert_eq!(first.image_id, "frame-a");
        assert!(gallery.note("cam-a", &["frame-a".to_owned()]).is_none());
        assert!(gallery.accept(
            &first,
            Thumb {
                url: "data:image/png;base64,AAAA".to_owned(),
                withheld: String::new(),
            },
        ));
        let second = gallery.pump().unwrap();
        assert_eq!(second.image_id, "frame-b");
        gallery.retarget("cam-b");
        assert!(gallery.get("frame-a").is_none());
        assert!(!gallery.accept(
            &second,
            Thumb {
                url: "late".to_owned(),
                withheld: String::new(),
            },
        ));
        let next = gallery.note("cam-b", &["other".to_owned()]).unwrap();
        assert_eq!(next.source_id, "cam-b");
        assert_eq!(next.image_id, "other");
    }

    fn shown(media: &str, data: &str, id: &str, caption: &str) -> Frame {
        interpret_frame(
            &page_reply(image_body_caption("get_image", media, data, id, caption)),
            None,
        )
        .unwrap()
    }

    fn shown_secret(media: &str, data: &str, secret: &str) -> Frame {
        let mut body = image_body("get_image", media, data);
        body["result"]["descriptor"]["caption"] = json!(secret);
        interpret_frame(&page_reply(body), Some(secret)).unwrap()
    }

    fn image_body(operation: &str, media: &str, data: &str) -> Value {
        image_body_caption(operation, media, data, "frame-a", "plate")
    }

    fn image_body_caption(
        operation: &str,
        media: &str,
        data: &str,
        id: &str,
        caption: &str,
    ) -> Value {
        json!({
            "operation": operation,
            "result": {
                "descriptor": {
                    "id": id,
                    "source_id": "cam-a",
                    "captured_at": "2024-01-01T00:00:00Z",
                    "media_type": media,
                    "width": 8,
                    "height": 4,
                    "caption": caption,
                    "attributes": {"slot": "1"}
                },
                "data": data
            }
        })
    }

    fn sample_frame(id: &str) -> Frame {
        let preview = crate::workbench::report::preview_for("image/png", "AAAA");
        Frame {
            id: id.to_owned(),
            source_id: "cam-a".to_owned(),
            captured_at: "2024-01-01T00:00:00Z".to_owned(),
            media_type: "image/png".to_owned(),
            width: 8,
            height: 4,
            caption: String::new(),
            attributes: String::new(),
            preview,
        }
    }

    fn page_reply(data: Value) -> Reply {
        Reply {
            status: 200,
            body: json!({
                "task": {
                    "id": "protocol-task",
                    "contextId": "protocol-context",
                    "status": {"state": "TASK_STATE_COMPLETED"},
                    "artifacts": [{
                        "artifactId": "art-0",
                        "parts": [{"data": data, "mediaType": "application/json"}]
                    }]
                }
            })
            .to_string(),
        }
    }
}
