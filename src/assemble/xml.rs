//! quick-xml event machinery: parse a part into owned events, edit spans, write it back.

use std::borrow::Cow;

use quick_xml::{
    Reader, Writer,
    events::{BytesStart, Event, attributes::Attribute},
    name::QName,
};

/// Parses an XML part into owned events (see the module docs for why the round-trip must be
/// byte-exact for untouched regions).
pub(super) fn parse_events(xml: &[u8], part: &str) -> Result<Vec<Event<'static>>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut events = Vec::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("{part} is not well-formed XML: {error}"))?;
        match event {
            Event::Eof => return Ok(events),
            event => events.push(event.into_owned()),
        }
    }
}

pub(super) fn element_is(event: &Event<'_>, name: &str) -> bool {
    match event {
        Event::Start(start) | Event::Empty(start) => start.local_name().into_inner() == name,
        _ => false,
    }
}

/// Serializes events back out; every untouched event is written with its raw parsed bytes.
pub(super) fn write_events(events: &[Event<'static>]) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    let mut writer = Writer::new(&mut buffer);
    for event in events {
        writer
            .write_event(event.borrow())
            .map_err(|error| format!("cannot serialize XML: {error}"))?;
    }
    Ok(buffer)
}

/// Span of the document's root element: `(start event index, exclusive end)`.
pub(super) fn root_span(events: &[Event<'static>]) -> Result<(usize, usize), String> {
    let start = events
        .iter()
        .position(|event| matches!(event, Event::Start(_)))
        .ok_or_else(|| "document has no root element".to_owned())?;
    let mut depth = 0usize;
    for (index, event) in events.iter().enumerate().skip(start + 1) {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                if depth == 0 {
                    return Ok((start, index + 1));
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    Err("document has no root element".to_owned())
}

/// Direct-child element spans of the element starting at `container_start`, whose exclusive
/// span end is `container_end` (its End event sits at `container_end - 1`). Returns
/// `(start, exclusive end)` pairs in document order; a Start..End subtree is covered whole so
/// a clone deep-copies children exactly like ElementTree's `copy.deepcopy`.
pub(super) fn child_spans(
    events: &[Event<'static>],
    container_start: usize,
    container_end: usize,
) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let last = container_end.saturating_sub(1);
    let mut index = container_start + 1;
    while index < last {
        match &events[index] {
            Event::Empty(_) => {
                spans.push((index, index + 1));
                index += 1;
            }
            Event::Start(_) => {
                let mut depth = 0usize;
                let mut cursor = index + 1;
                let end = loop {
                    if cursor >= last {
                        break last; // malformed input: treat the rest as the child's body
                    }
                    match &events[cursor] {
                        Event::Start(_) => depth += 1,
                        Event::End(_) if depth == 0 => break cursor + 1,
                        Event::End(_) => depth -= 1,
                        _ => {}
                    }
                    cursor += 1;
                };
                spans.push((index, end));
                index = end;
            }
            _ => index += 1,
        }
    }
    spans
}

pub(super) fn attribute_value(start: &BytesStart<'_>, key: &str) -> Result<Option<String>, String> {
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| format!("malformed attribute in XML part: {error}"))?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.value.into_owned()));
        }
    }
    Ok(None)
}

/// Sets (or adds) an attribute while preserving the element's attribute order: quick-xml has
/// no in-place attribute editing, so the whole list is rebuilt. Raw values are kept in their
/// parsed (escaped) form; generated values (numbers, UUIDs, paths) contain no characters that
/// would need escaping.
pub(super) fn set_attribute(
    start: &mut BytesStart<'static>,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let mut attributes: Vec<(String, String)> = Vec::new();
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| format!("malformed attribute in XML part: {error}"))?;
        attributes.push((
            attribute.key.as_ref().to_owned(),
            attribute.value.into_owned(),
        ));
    }
    match attributes.iter_mut().find(|(name, _)| name == key) {
        Some((_, existing)) => *existing = value.to_owned(),
        None => attributes.push((key.to_owned(), value.to_owned())),
    }
    start.clear_attributes();
    for (name, value) in &attributes {
        start.push_attribute(Attribute {
            key: QName(name.as_str()),
            value: Cow::Borrowed(value.as_str()),
        });
    }
    Ok(())
}

pub(super) fn stack_ends_with(stack: &[String], suffix: &[&str]) -> bool {
    stack.len() >= suffix.len()
        && stack[stack.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(entry, expected)| entry.as_str() == *expected)
}
