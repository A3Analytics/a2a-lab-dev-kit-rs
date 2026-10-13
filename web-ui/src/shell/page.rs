//! Text-only DOM helpers. Agent strings are assigned with `set_text_content`.

use std::cell::RefCell;
use std::collections::BTreeMap;

use wasm_bindgen::JsCast;
use web_sys::{
    Document, Element, HtmlElement, HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement,
};

std::thread_local! {
    static NODES: RefCell<BTreeMap<String, Element>> = const { RefCell::new(BTreeMap::new()) };
}

#[derive(Clone)]
pub struct Page {
    document: Document,
}

impl Page {
    pub fn bind() -> Option<Self> {
        let document = web_sys::window()?.document()?;
        Some(Self { document })
    }

    pub fn mount_root(&self) -> Option<Element> {
        self.document.get_element_by_id("app")
    }

    pub fn create(&self, tag: &str) -> Option<Element> {
        self.document.create_element(tag).ok()
    }

    pub fn element(&self, id: &str) -> Option<Element> {
        self.document
            .get_element_by_id(id)
            .or_else(|| remembered(id))
    }

    pub fn set_text(&self, id: &str, text: &str) {
        if let Some(element) = self.element(id) {
            element.set_text_content(Some(text));
        }
    }

    pub fn set_data(&self, id: &str, name: &str, value: &str) {
        if let Some(element) = self.element(id) {
            let _ = element.set_attribute(name, value);
        }
    }

    pub fn data(&self, id: &str, name: &str) -> String {
        self.element(id)
            .and_then(|element| element.get_attribute(name))
            .unwrap_or_default()
    }

    pub fn clear(&self, id: &str) {
        if let Some(element) = self.element(id) {
            clear_children(&element);
        }
    }

    pub fn control_value(&self, id: &str) -> String {
        let Some(element) = self.element(id) else {
            return String::new();
        };
        if let Ok(input) = element.clone().dyn_into::<HtmlInputElement>() {
            return input.value();
        }
        if let Ok(select) = element.clone().dyn_into::<HtmlSelectElement>() {
            return select.value();
        }
        element
            .dyn_into::<HtmlTextAreaElement>()
            .map(|area| area.value())
            .unwrap_or_default()
    }

    pub fn remove_attr(&self, id: &str, name: &str) {
        if let Some(element) = self.element(id) {
            let _ = element.remove_attribute(name);
        }
    }

    pub fn focus(&self, id: &str) {
        let Some(element) = self.element(id) else {
            return;
        };
        if let Ok(html) = element.dyn_into::<HtmlElement>() {
            let _ = html.focus();
        }
    }

    pub fn set_control(&self, id: &str, value: &str) {
        let Some(element) = self.element(id) else {
            return;
        };
        if let Ok(input) = element.clone().dyn_into::<HtmlInputElement>() {
            input.set_value(value);
            return;
        }
        if let Ok(select) = element.clone().dyn_into::<HtmlSelectElement>() {
            select.set_value(value);
            return;
        }
        if let Ok(area) = element.dyn_into::<HtmlTextAreaElement>() {
            area.set_value(value);
        }
    }

    pub fn secret(&self) -> Option<String> {
        let value = self.control_value("bearer-token");
        (!value.is_empty()).then_some(value)
    }
}

pub fn remember(id: &str, element: &Element) {
    NODES.with(|nodes| {
        nodes.borrow_mut().insert(id.to_owned(), element.clone());
    });
}

fn remembered(id: &str) -> Option<Element> {
    NODES.with(|nodes| nodes.borrow().get(id).cloned())
}

pub fn clear_children(element: &Element) {
    while let Some(child) = element.first_child() {
        let _ = element.remove_child(&child);
    }
}

pub fn attr(element: &Element, name: &str, value: &str) {
    let _ = element.set_attribute(name, value);
}
