//! Task-definition reader. The sidebar lists definitions; the main column shows details and the generated input form.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::Event;

use std::cell::RefCell;

use crate::inventory::{Item, Primitive};
use crate::shell::http::Transport;
use crate::shell::page::{self, Page};
use crate::shell::present;
use crate::tasks::{self as schema, Field, Form, Kind};

pub fn mount(page: &Page, parent: &web_sys::Element) {
    present::section(page, parent, "task-view", "Tasks", |body| {
        let status = present::node(page, "p", Some("task-view-status"));
        page::attr(&status, "role", "status");
        page::attr(&status, "aria-live", "polite");
        body.append_child(&status).ok();
        let alert = present::node(page, "div", Some("task-view-alert"));
        page::attr(&alert, "role", "alert");
        page::attr(&alert, "aria-live", "assertive");
        body.append_child(&alert).ok();
        let browser = present::node(page, "div", Some("task-browser"));
        page::attr(&browser, "class", "log-browser");
        let filters = present::node(page, "div", Some("task-filters"));
        page::attr(&filters, "role", "group");
        page::attr(&filters, "aria-label", "Task filters");
        let source_group = present::node(page, "div", None);
        page::attr(&source_group, "class", "log-filter-group");
        let source_label = present::node(page, "p", None);
        source_label.set_text_content(Some("Task"));
        let sources = present::node(page, "div", Some("task-sources"));
        page::attr(&sources, "role", "radiogroup");
        page::attr(&sources, "aria-label", "Task definitions");
        source_group.append_child(&source_label).ok();
        source_group.append_child(&sources).ok();
        filters.append_child(&source_group).ok();
        let results = present::node(page, "div", Some("task-results"));
        page::attr(&results, "class", "log-results");
        let scroll = present::node(page, "div", None);
        page::attr(&scroll, "class", "log-scroll");
        let detail = present::node(page, "dl", Some("task-detail"));
        page::attr(&detail, "class", "task-detail");
        scroll.append_child(&detail).ok();
        let form = present::node(page, "form", Some("task-form"));
        page::attr(&form, "class", "task-form");
        page::attr(&form, "aria-label", "Task input");
        page::attr(&form, "novalidate", "novalidate");
        scroll.append_child(&form).ok();
        let runs = present::node(page, "ul", Some("task-runs"));
        page::attr(&runs, "class", "log-cards");
        page::attr(&runs, "aria-label", "Task runs");
        scroll.append_child(&runs).ok();
        results.append_child(&scroll).ok();
        browser.append_child(&filters).ok();
        browser.append_child(&results).ok();
        body.append_child(&browser).ok();
    });
    fill_sources(page, &[]);
    page.set_data("task-view-status", "data-gen", "0");
    page.set_text("task-view-status", "Choose a task.");
}

pub fn install(page: &Page, transport: &Transport) {
    super::task_runs::install(transport);
    on_sources(page);
    on_form(page);
}

pub fn catalog_changed(page: &Page) {
    let items = super::inventory::loaded(Primitive::Tasks);
    let previous = selected();
    fill_sources(page, &items);
    let next = if items.iter().any(|item| item.id == previous) {
        previous
    } else {
        items
            .first()
            .map(|item| item.id.clone())
            .unwrap_or_default()
    };
    choose(page, &next);
    if next.is_empty() {
        clear_definition(page);
        copy_catalog_state(page);
        return;
    }
    show_definition(page, &next);
}

fn fill_sources(page: &Page, items: &[Item]) {
    let Some(list) = page.element("task-sources") else {
        return;
    };
    page::clear_children(&list);
    if items.is_empty() {
        let button = present::node(page, "button", Some("task-pick-0"));
        page::attr(&button, "type", "button");
        page::attr(&button, "class", "task-source");
        page::attr(&button, "role", "radio");
        page::attr(&button, "aria-checked", "false");
        page::attr(&button, "aria-disabled", "true");
        button.set_text_content(Some("None loaded"));
        list.append_child(&button).ok();
        return;
    }
    let current = selected();
    for (index, item) in items.iter().enumerate() {
        let button = present::node(page, "button", Some(&format!("task-pick-{index}")));
        page::attr(&button, "type", "button");
        page::attr(&button, "class", "task-source");
        page::attr(&button, "role", "radio");
        page::attr(&button, "data-task", &item.id);
        page::attr(
            &button,
            "aria-checked",
            if item.id == current { "true" } else { "false" },
        );
        let name = present::node(page, "span", None);
        page::attr(&name, "class", "task-source-name");
        name.set_text_content(Some(if item.name.is_empty() {
            &item.id
        } else {
            &item.name
        }));
        let detail = present::node(page, "span", None);
        page::attr(&detail, "class", "task-source-detail");
        detail.set_text_content(Some(&source_detail(item)));
        button.append_child(&name).ok();
        button.append_child(&detail).ok();
        list.append_child(&button).ok();
    }
}

fn source_detail(item: &Item) -> String {
    if item.description.is_empty() {
        item.id.clone()
    } else {
        format!("{} · {}", item.id, item.description)
    }
}

fn on_sources(page: &Page) {
    let Some(sources) = page.element("task-sources") else {
        return;
    };
    let page_for_click = page.clone();
    let click = Closure::wrap(Box::new(move |event: Event| {
        let Some(id) = event
            .target()
            .and_then(|target| target.dyn_ref::<web_sys::Element>().cloned())
            .and_then(|element| element.closest("button").ok().flatten())
            .and_then(|button| button.get_attribute("data-task"))
        else {
            return;
        };
        if id.is_empty() || id == selected() {
            return;
        }
        choose(&page_for_click, &id);
        show_definition(&page_for_click, &id);
    }) as Box<dyn FnMut(Event)>);
    let _ = sources.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
    click.forget();
    let page_for_key = page.clone();
    let key = Closure::wrap(Box::new(move |event: Event| {
        let Some(key_event) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        let delta: isize = match key_event.key().as_str() {
            "ArrowDown" | "ArrowRight" => 1,
            "ArrowUp" | "ArrowLeft" => -1,
            _ => return,
        };
        key_event.prevent_default();
        let Some(id) = neighbor(delta) else {
            return;
        };
        if id == selected() {
            return;
        }
        choose(&page_for_key, &id);
        focus_selected(&page_for_key);
        show_definition(&page_for_key, &id);
    }) as Box<dyn FnMut(Event)>);
    let _ = sources.add_event_listener_with_callback("keydown", key.as_ref().unchecked_ref());
    key.forget();
}

fn on_form(page: &Page) {
    let Some(form) = page.element("task-form") else {
        return;
    };
    let input = Closure::wrap(Box::new(move |event: Event| {
        commit_field(&event);
    }) as Box<dyn FnMut(Event)>);
    let _ = form.add_event_listener_with_callback("input", input.as_ref().unchecked_ref());
    input.forget();
    let change = Closure::wrap(Box::new(move |event: Event| {
        commit_field(&event);
    }) as Box<dyn FnMut(Event)>);
    let _ = form.add_event_listener_with_callback("change", change.as_ref().unchecked_ref());
    change.forget();
    let page_for_click = page.clone();
    let click = Closure::wrap(Box::new(move |event: Event| {
        let Some(button) = event
            .target()
            .and_then(|target| target.dyn_ref::<web_sys::Element>().cloned())
            .and_then(|element| element.closest("button").ok().flatten())
        else {
            return;
        };
        if let Some(path) = button.get_attribute("data-task-add") {
            event.prevent_default();
            FORM.with(|slot| {
                if let Some(form) = slot.borrow_mut().as_mut() {
                    form.add(&path);
                }
            });
            paint_form(&page_for_click);
            return;
        }
        if let Some(path) = button.get_attribute("data-task-remove") {
            event.prevent_default();
            FORM.with(|slot| {
                if let Some(form) = slot.borrow_mut().as_mut() {
                    form.remove(&path);
                }
            });
            paint_form(&page_for_click);
        }
    }) as Box<dyn FnMut(Event)>);
    let _ = form.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
    click.forget();
    let page_for_submit = page.clone();
    let submit = Closure::wrap(Box::new(move |event: Event| {
        event.prevent_default();
        start_selected(&page_for_submit);
    }) as Box<dyn FnMut(Event)>);
    let _ = form.add_event_listener_with_callback("submit", submit.as_ref().unchecked_ref());
    submit.forget();
}

fn commit_field(event: &Event) {
    let Some(element) = event
        .target()
        .and_then(|target| target.dyn_ref::<web_sys::Element>().cloned())
    else {
        return;
    };
    let Some(path) = element.get_attribute("data-task-path") else {
        return;
    };
    let value = if let Ok(input) = element.clone().dyn_into::<web_sys::HtmlInputElement>() {
        if input.type_() == "checkbox" {
            if input.checked() {
                "true".to_owned()
            } else {
                "false".to_owned()
            }
        } else {
            input.value()
        }
    } else if let Ok(select) = element.clone().dyn_into::<web_sys::HtmlSelectElement>() {
        select.value()
    } else if let Ok(area) = element.dyn_into::<web_sys::HtmlTextAreaElement>() {
        area.value()
    } else {
        return;
    };
    FORM.with(|slot| {
        if let Some(form) = slot.borrow_mut().as_mut() {
            form.set(&path, &value);
        }
    });
}

fn neighbor(delta: isize) -> Option<String> {
    let items = super::inventory::loaded(Primitive::Tasks);
    if items.is_empty() {
        return None;
    }
    let current = selected();
    let index = items
        .iter()
        .position(|item| item.id == current)
        .unwrap_or(0);
    let next = (index as isize + delta).rem_euclid(items.len() as isize) as usize;
    Some(items[next].id.clone())
}

fn choose(page: &Page, id: &str) {
    SELECTED.with(|slot| *slot.borrow_mut() = id.to_owned());
    let items = super::inventory::loaded(Primitive::Tasks);
    for (index, item) in items.iter().enumerate() {
        page.set_data(
            &format!("task-pick-{index}"),
            "aria-checked",
            if item.id == id { "true" } else { "false" },
        );
    }
}

fn focus_selected(page: &Page) {
    let items = super::inventory::loaded(Primitive::Tasks);
    let current = selected();
    if let Some(index) = items.iter().position(|item| item.id == current) {
        page.focus(&format!("task-pick-{index}"));
    }
}

fn show_definition(page: &Page, id: &str) {
    let items = super::inventory::loaded(Primitive::Tasks);
    let Some(item) = items.into_iter().find(|item| item.id == id) else {
        clear_definition(page);
        copy_catalog_state(page);
        return;
    };
    render_detail(page, &item);
    let secret = page.secret();
    remember_form(schema::form_for(&item, secret.as_deref()));
    paint_form(page);
    page.set_text("task-view-status", "");
    page.set_text("task-view-alert", "");
    page.set_data("task-view", "aria-busy", "false");
}

fn render_detail(page: &Page, item: &Item) {
    let Some(detail) = page.element("task-detail") else {
        return;
    };
    page::clear_children(&detail);
    fact(page, &detail, "task-detail-id", "Id", &item.id);
    fact(page, &detail, "task-detail-name", "Name", &item.name);
    fact(
        page,
        &detail,
        "task-detail-description",
        "Description",
        &item.description,
    );
    fact(
        page,
        &detail,
        "task-detail-asset-id",
        "Asset id",
        &named(&item.details, "Asset id"),
    );
    fact(
        page,
        &detail,
        "task-detail-semantic-id",
        "Semantic id",
        &named(&item.details, "Semantic id"),
    );
    fact(
        page,
        &detail,
        "task-detail-input-schema",
        "Input schema",
        &named(&item.details, "Input schema"),
    );
    fact(
        page,
        &detail,
        "task-detail-output-schema",
        "Output schema",
        &named(&item.details, "Output schema"),
    );
}

fn fact(page: &Page, parent: &web_sys::Element, id: &str, label: &str, value: &str) {
    let term = present::node(page, "dt", None);
    term.set_text_content(Some(label));
    let data = present::node(page, "dd", Some(id));
    data.set_text_content(Some(value));
    parent.append_child(&term).ok();
    parent.append_child(&data).ok();
}

fn named(details: &[(&'static str, String)], label: &str) -> String {
    details
        .iter()
        .find(|(name, _)| *name == label)
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

fn paint_form(page: &Page) {
    let Some(root) = page.element("task-form") else {
        return;
    };
    page::clear_children(&root);
    let Some(form) = remembered_form() else {
        return;
    };
    if form.needs_input() {
        draw_field(page, &root, &form.root);
    } else {
        let empty = present::node(page, "p", Some("task-form-empty"));
        empty.set_text_content(Some("This task has no input fields."));
        root.append_child(&empty).ok();
    }
    let start = present::node(page, "button", Some("task-start"));
    page::attr(&start, "type", "submit");
    start.set_text_content(Some("Start"));
    root.append_child(&start).ok();
}

fn draw_field(page: &Page, parent: &web_sys::Element, field: &Field) {
    match field.kind {
        Kind::Object => draw_object(page, parent, field),
        Kind::Array => draw_array(page, parent, field),
        Kind::Enum => draw_enum(page, parent, field),
        Kind::Json => draw_json(page, parent, field),
        Kind::Boolean => draw_checkbox(page, parent, field),
        _ => draw_input(page, parent, field),
    }
}

fn draw_object(page: &Page, parent: &web_sys::Element, field: &Field) {
    let group = present::node(page, "fieldset", None);
    page::attr(&group, "class", "task-field-group");
    let legend = present::node(page, "legend", None);
    legend.set_text_content(Some(&field.label));
    group.append_child(&legend).ok();
    help(page, &group, field);
    for child in &field.children {
        draw_field(page, &group, child);
    }
    parent.append_child(&group).ok();
}

fn draw_array(page: &Page, parent: &web_sys::Element, field: &Field) {
    let group = present::node(page, "fieldset", None);
    page::attr(&group, "class", "task-field-group");
    let legend = present::node(page, "legend", None);
    legend.set_text_content(Some(&field.label));
    group.append_child(&legend).ok();
    help(page, &group, field);
    for child in &field.children {
        let item = present::node(page, "div", None);
        page::attr(&item, "class", "task-array-item");
        draw_field(page, &item, child);
        let remove = present::node(page, "button", None);
        page::attr(&remove, "type", "button");
        page::attr(&remove, "data-task-remove", &child.path);
        page::attr(&remove, "aria-label", &format!("Remove {}", child.label));
        remove.set_text_content(Some("Remove"));
        item.append_child(&remove).ok();
        group.append_child(&item).ok();
    }
    let add = present::node(page, "button", None);
    page::attr(&add, "type", "button");
    page::attr(&add, "data-task-add", &field.path);
    page::attr(&add, "aria-label", &format!("Add {}", field.label));
    add.set_text_content(Some("Add item"));
    group.append_child(&add).ok();
    parent.append_child(&group).ok();
}

fn draw_enum(page: &Page, parent: &web_sys::Element, field: &Field) {
    let wrap = wrap_control(page, parent, field);
    let select = present::node(page, "select", Some(&field.id()));
    page::attr(&select, "data-task-path", &field.path);
    apply_attrs(&select, field);
    describe(&select, field);
    let blank = present::node(page, "option", None);
    page::attr(&blank, "value", "");
    blank.set_text_content(Some("Choose"));
    select.append_child(&blank).ok();
    for option in &field.options {
        let label = option_label(option);
        let node = present::node(page, "option", None);
        page::attr(&node, "value", &label);
        if field.raw == label {
            page::attr(&node, "selected", "selected");
        }
        node.set_text_content(Some(&label));
        select.append_child(&node).ok();
    }
    wrap.append_child(&select).ok();
    error(page, &wrap, field);
}

fn draw_json(page: &Page, parent: &web_sys::Element, field: &Field) {
    let wrap = wrap_control(page, parent, field);
    let area = present::node(page, "textarea", Some(&field.id()));
    page::attr(&area, "data-task-path", &field.path);
    page::attr(&area, "rows", "6");
    apply_attrs(&area, field);
    describe(&area, field);
    if let Ok(control) = area.clone().dyn_into::<web_sys::HtmlTextAreaElement>() {
        control.set_value(&field.raw);
    }
    wrap.append_child(&area).ok();
    error(page, &wrap, field);
}

fn draw_checkbox(page: &Page, parent: &web_sys::Element, field: &Field) {
    let wrap = present::node(page, "div", None);
    page::attr(&wrap, "class", "task-field task-field-check");
    let input = present::node(page, "input", Some(&field.id()));
    page::attr(&input, "data-task-path", &field.path);
    apply_attrs(&input, field);
    describe(&input, field);
    if field.raw == "true" {
        page::attr(&input, "checked", "checked");
    }
    let label = present::node(page, "label", None);
    page::attr(&label, "for", &field.id());
    label.set_text_content(Some(&field.label));
    wrap.append_child(&input).ok();
    wrap.append_child(&label).ok();
    help(page, &wrap, field);
    error(page, &wrap, field);
    parent.append_child(&wrap).ok();
}

fn draw_input(page: &Page, parent: &web_sys::Element, field: &Field) {
    let wrap = wrap_control(page, parent, field);
    let input = present::node(page, "input", Some(&field.id()));
    page::attr(&input, "data-task-path", &field.path);
    apply_attrs(&input, field);
    describe(&input, field);
    if let Ok(control) = input.clone().dyn_into::<web_sys::HtmlInputElement>() {
        control.set_value(&field.raw);
    }
    wrap.append_child(&input).ok();
    error(page, &wrap, field);
}

fn wrap_control(page: &Page, parent: &web_sys::Element, field: &Field) -> web_sys::Element {
    let wrap = present::node(page, "div", None);
    page::attr(&wrap, "class", "task-field");
    let label = present::node(page, "label", None);
    page::attr(&label, "for", &field.id());
    label.set_text_content(Some(&field.label));
    wrap.append_child(&label).ok();
    help(page, &wrap, field);
    parent.append_child(&wrap).ok();
    wrap
}

fn help(page: &Page, parent: &web_sys::Element, field: &Field) {
    if field.description.is_empty() {
        return;
    }
    let help = present::node(page, "p", Some(&field.help_id()));
    page::attr(&help, "class", "task-field-help");
    help.set_text_content(Some(&field.description));
    parent.append_child(&help).ok();
}

fn error(page: &Page, parent: &web_sys::Element, field: &Field) {
    let Some(message) = &field.error else {
        return;
    };
    let error = present::node(page, "p", Some(&field.error_id()));
    page::attr(&error, "class", "task-field-error");
    error.set_text_content(Some(message));
    parent.append_child(&error).ok();
}

fn apply_attrs(element: &web_sys::Element, field: &Field) {
    for (name, value) in schema::attrs(field) {
        page::attr(element, name, &value);
    }
}

fn describe(element: &web_sys::Element, field: &Field) {
    let mut described = Vec::new();
    if !field.description.is_empty() {
        described.push(field.help_id());
    }
    if field.error.is_some() {
        described.push(field.error_id());
    }
    if !described.is_empty() {
        page::attr(element, "aria-describedby", &described.join(" "));
    }
}

fn option_label(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn copy_catalog_state(page: &Page) {
    let spec = Primitive::Tasks.spec();
    let kind = page.data(spec.status, "data-state");
    let status = page
        .element(spec.status)
        .and_then(|element| element.text_content())
        .unwrap_or_default();
    let alert = page
        .element(spec.alert)
        .and_then(|element| element.text_content())
        .unwrap_or_default();
    page.set_data(
        "task-view",
        "aria-busy",
        if kind == "loading" { "true" } else { "false" },
    );
    page.set_text("task-view-status", &status);
    page.set_text("task-view-alert", &alert);
}

fn clear_definition(page: &Page) {
    remember_form(schema::form(None, None));
    if let Some(detail) = page.element("task-detail") {
        page::clear_children(&detail);
    }
    if let Some(form) = page.element("task-form") {
        page::clear_children(&form);
    }
}

fn selected() -> String {
    SELECTED.with(|slot| slot.borrow().clone())
}

fn remember_form(form: Form) {
    FORM.with(|slot| *slot.borrow_mut() = Some(form));
}

fn remembered_form() -> Option<Form> {
    FORM.with(|slot| slot.borrow().clone())
}

fn start_selected(page: &Page) {
    let task_id = selected();
    if task_id.is_empty() {
        return;
    }
    let input = FORM.with(|slot| slot.borrow_mut().as_mut().and_then(Form::submit));
    let Some(input) = input else {
        paint_form(page);
        return;
    };
    paint_form(page);
    super::task_runs::start(page, task_id, input);
}

std::thread_local! {
    static SELECTED: RefCell<String> = const { RefCell::new(String::new()) };
    static FORM: RefCell<Option<Form>> = const { RefCell::new(None) };
}
