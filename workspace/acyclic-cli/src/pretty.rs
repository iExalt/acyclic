//! Output for people, from the same data the JSON envelope carries: tables for lists, a block of fields for each
//! record, and one line for a write. Fitted to the terminal's width when printing to one, and without borders or
//! wrapping when not, so it stays easy to pipe. Records are named as a person names them: items and threads by the
//! shortest ending of their ids that names them, and repositories by `owner/name`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use comfy_table::{ContentArrangement, LineStyle, Table, TableStyle, presets};
use serde_json::Value as Json;

use crate::args::{Command, ItemCommand, ThreadCommand};

/// A terminal's table: a rule under the header and nothing else, so a wrapped cell still reads as one row.
const RULED: TableStyle =
    TableStyle::new().header_separator(LineStyle::none().fill('─').junction('─'));

/// The entities a write names, in the order a summary looks for them.
const ENTITIES: [&str; 3] = ["item", "thread", "repository"];

/// A command's data for people, each id that `names` holds printed as its name there. `width` is the terminal's, or
/// `None` when not printing to one.
pub(crate) fn render(
    command: &Command,
    data: &Json,
    width: Option<u16>,
    names: &BTreeMap<String, String>,
) -> String {
    let data = &named(data, names);
    let columns = |keys: &[&str]| keys.iter().map(|key| (*key).to_owned()).collect::<Vec<_>>();
    let text = match command {
        Command::Item(ItemCommand::List) => table(
            rows(&data["items"]),
            &columns(&["id", "title", "state", "thread", "readiness"]),
            width,
        ),
        Command::Thread(ThreadCommand::Ready) => ready(data, width),
        Command::Thread(ThreadCommand::List) => threads(data, width),
        Command::Import { .. } => import(data),
        Command::Init { .. } => format!(
            "{} tracker {} · replica {}",
            if data["created"] == true {
                "created"
            } else if data["changed"] == true {
                "changed"
            } else {
                "no change to"
            },
            compact(&data["path"]),
            compact(&data["replica"])
        ),
        Command::Sync => format!(
            "synced tracker with {} · fetched {} · joined {} · pushed {}",
            compact(&data["remote"]),
            compact(&data["fetched"]),
            compact(&data["joined"]),
            compact(&data["pushed"])
        ),
        Command::Item(ItemCommand::Add { waits_for, .. }) => waits(write(data), waits_for, &[]),
        Command::Item(ItemCommand::Edit(edit)) => {
            waits(write(data), &edit.waits_for, &edit.no_waits_for)
        },
        _ => match data {
            Json::Array(records) => {
                records.iter().map(|record| block(record, width)).collect::<Vec<_>>().join("\n")
            },
            Json::Object(fields) if fields.contains_key("changed") => write(data),
            _ => block(data, width),
        },
    };
    let mut text = if text.ends_with('\n') { text } else { format!("{text}\n") };
    for unchecked in rows(&data["unchecked_commits"]) {
        let _ = writeln!(
            text,
            "warning: {}@{} isn't checked on this machine: {}",
            compact(&unchecked["repository"]),
            compact(&unchecked["commit"]),
            compact(&unchecked["reason"])
        );
    }
    text
}

/// `value` with every string, and every key, that is an id in `names` replaced by its name.
fn named(value: &Json, names: &BTreeMap<String, String>) -> Json {
    let name = |text: &String| names.get(text).unwrap_or(text).clone();
    match value {
        Json::String(text) => Json::String(name(text)),
        Json::Array(values) => values.iter().map(|value| named(value, names)).collect(),
        Json::Object(fields) => {
            fields.iter().map(|(key, value)| (name(key), named(value, names))).collect()
        },
        other => other.clone(),
    }
}

/// An error's envelope as one line for standard error, with its details when it has any, since they often name what
/// to act on.
pub(crate) fn error(envelope: &Json) -> String {
    let error = &envelope["error"];
    let message = error["message"].as_str().unwrap_or_default();
    let code = error["code"].as_str().unwrap_or_default();
    let details = &error["details"];
    if empty(details) {
        format!("error: {message} [{code}]\n")
    } else {
        format!("error: {message} [{code}] ({})\n", compact(details))
    }
}

/// An import as one line: what it wrote, or would write on a dry run, by kind, and the threads items joined.
fn import(data: &Json) -> String {
    let dry = data["dry_run"] == true;
    let kinds = [
        ("item", "item", "items"),
        ("thread", "thread", "threads"),
        ("link", "waits-for", "waits-for"),
        ("repository", "repository", "repositories"),
    ];
    let changes = rows(&data["changes"]);
    let mut written = Vec::new();
    for (action, past) in [("create", "created"), ("update", "updated"), ("remove", "removed")] {
        let counts: Vec<String> = kinds
            .iter()
            .filter_map(|(kind, one, many)| {
                let n = changes
                    .iter()
                    .filter(|change| change["record"] == *kind && change["action"] == action)
                    .count();
                (n > 0).then(|| format!("{n} {}", if n == 1 { one } else { many }))
            })
            .collect();
        if !counts.is_empty() {
            written.push(format!("{} {}", if dry { action } else { past }, counts.join(", ")));
        }
    }
    let mut line = if written.is_empty() {
        String::new()
    } else {
        let fields = &data["counts"]["fields"];
        format!(
            "{}: {} ({} {})",
            if dry { "would import" } else { "imported" },
            written.join("; "),
            compact(fields),
            if fields == 1 { "field" } else { "fields" }
        )
    };
    if !empty(&data["placed"]) {
        if !line.is_empty() {
            line.push_str(" · ");
        }
        write!(line, "{} {}", if dry { "would place" } else { "placed" }, compact(&data["placed"]))
            .expect("a String takes writes");
    }
    if line.is_empty() {
        line.push_str(if dry { "would make no change" } else { "no change" });
    }
    line
}

/// The objects in a list.
fn rows(list: &Json) -> Vec<&Json> {
    list.as_array().map(|rows| rows.iter().collect()).unwrap_or_default()
}

/// Rows as a table under the given columns, or `none` for no rows.
fn table(rows: Vec<&Json>, columns: &[String], width: Option<u16>) -> String {
    if rows.is_empty() {
        return "none\n".to_owned();
    }
    let mut table = Table::new();
    match width {
        Some(width) => {
            table
                .load_style(RULED)
                .set_content_arrangement(ContentArrangement::Dynamic)
                .set_width(width);
        },
        None => {
            table
                .load_style(presets::NOTHING)
                .set_content_arrangement(ContentArrangement::Disabled);
        },
    }
    table.set_header(columns.iter().map(|column| column.to_uppercase()));
    for row in rows {
        table.add_row(columns.iter().map(|column| {
            if row["kind"] == "conflict" && column == "values" {
                conflict_values(&row[column])
            } else {
                compact(&row[column])
            }
        }));
    }
    let mut text = table.to_string();
    if width.is_none() {
        // Without borders, the padding at each line's end is noise in a pipe.
        text = text
            .lines()
            .map(|line| line.strip_prefix(' ').unwrap_or(line).trim_end())
            .collect::<Vec<_>>()
            .join("\n");
    }
    text.push('\n');
    text
}

/// `thread ready`: one row per ready item, under its thread, the thread named on its first row.
fn ready(data: &Json, width: Option<u16>) -> String {
    let mut lines = Vec::new();
    for thread in rows(&data["ready"]) {
        let mut first = true;
        for item in rows(&thread["items"]) {
            let named = if std::mem::take(&mut first) {
                format!("{} {}", compact(&thread["id"]), compact(&thread["title"]))
            } else {
                String::new()
            };
            lines.push(serde_json::json!({
                "thread": named, "item": item["id"], "title": item["title"],
            }));
        }
    }
    let columns: Vec<String> =
        ["thread", "item", "title"].iter().map(|key| (*key).to_owned()).collect();
    table(lines.iter().collect(), &columns, width)
}

/// `thread list`: each thread's summary, with its counts as columns, and its wave: a number, `any` for a thread that
/// can start any time, or nothing once its work is done.
fn threads(data: &Json, width: Option<u16>) -> String {
    let mut wave: BTreeMap<&str, String> = BTreeMap::new();
    for (i, ids) in rows(&data["waves"]).into_iter().enumerate() {
        wave.extend(
            rows(ids).into_iter().filter_map(Json::as_str).map(|id| (id, (i + 1).to_string())),
        );
    }
    wave.extend(
        rows(&data["anytime"])
            .into_iter()
            .filter_map(Json::as_str)
            .map(|id| (id, "any".to_owned())),
    );
    let lines: Vec<Json> = rows(&data["threads"])
        .into_iter()
        .map(|thread| {
            serde_json::json!({
                "id": thread["id"], "title": thread["title"],
                "members": thread["counts"]["members"], "done": thread["counts"]["done"], "heads": thread["heads"],
                "wave": thread["id"].as_str().and_then(|id| wave.get(id)),
            })
        })
        .collect();
    let columns: Vec<String> = ["id", "title", "members", "done", "wave", "heads"]
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    table(lines.iter().collect(), &columns, width)
}

/// A write as one line: what changed, and anything else it says that isn't empty.
fn write(data: &Json) -> String {
    let fields = data.as_object().expect("a write's data is an object");
    let entity = ENTITIES.iter().find(|entity| fields.get(**entity).is_some_and(Json::is_string));
    let changed = if data["changed"] == true { "changed" } else { "no change to" };
    // The entity's title, when the write carries it, reads as part of its name.
    let title = data["title"].as_str().filter(|_| entity.is_some());
    let mut parts = vec![match (entity, title) {
        (Some(entity), Some(title)) => {
            format!("{changed} {entity} {} {}", compact(&data[*entity]), Json::from(title))
        },
        (Some(entity), None) => format!("{changed} {entity} {}", compact(&data[*entity])),
        (None, _) => changed.to_owned(),
    }];
    for (key, value) in fields {
        if key == "changed"
            || entity.is_some_and(|entity| *entity == key.as_str())
            || (key == "title" && title.is_some())
            || empty(value)
        {
            continue;
        }
        parts.push(format!("{key} {}", compact(value)));
    }
    parts.join(" · ")
}

/// An item write's line, with what it now waits for and no longer waits for, as the command named them.
fn waits(line: String, waits_for: &[String], no_waits_for: &[String]) -> String {
    let mut parts = vec![line];
    if !waits_for.is_empty() {
        parts.push(format!("waits for {}", waits_for.join(", ")));
    }
    if !no_waits_for.is_empty() {
        parts.push(format!("no longer waits for {}", no_waits_for.join(", ")));
    }
    parts.join(" · ")
}

/// A record as one field per line, its names aligned, leaving out what is empty. A list of records inside it is a
/// table under its name.
fn block(record: &Json, width: Option<u16>) -> String {
    let Some(fields) = record.as_object() else {
        return compact(record);
    };
    // A document that is only a list, as `needs` is, is that list's table.
    if let [(_, list)] = fields.iter().collect::<Vec<_>>()[..]
        && (list.is_array())
    {
        return list_table(rows(list), width);
    }
    let shown: Vec<(&String, &Json)> = leading(fields.keys())
        .into_iter()
        .map(|key| (key, &fields[key]))
        .filter(|(_, value)| !empty(value))
        .collect();
    let (lists, scalars): (Vec<_>, Vec<_>) =
        shown.into_iter().partition(|(_, value)| records(value));
    let name_width = scalars.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    let mut text = String::new();
    for (key, value) in scalars {
        // A value of several lines, such as notes, keeps its later lines under its first.
        let value =
            compact(value).replace('\n', &format!("\n{:indent$}", "", indent = name_width + 2));
        writeln!(text, "{key:name_width$}  {value}").expect("a String takes writes");
    }
    for (key, value) in lists {
        write!(text, "\n{key}:\n{}", list_table(rows(value), width))
            .expect("a String takes writes");
    }
    text
}

/// Records of any shape as a table, with a column for every field any of them has.
fn list_table(rows: Vec<&Json>, width: Option<u16>) -> String {
    let mut keys: Vec<&String> = Vec::new();
    for row in &rows {
        for column in row.as_object().into_iter().flat_map(serde_json::Map::keys) {
            if !keys.contains(&column) {
                keys.push(column);
            }
        }
    }
    let columns: Vec<String> = leading(keys).into_iter().cloned().collect();
    table(rows, &columns, width)
}

/// Field names with `kind`, `record`, `id` and `title` first, as a record is named, then the rest in order.
fn leading<'a>(keys: impl IntoIterator<Item = &'a String>) -> Vec<&'a String> {
    let mut keys: Vec<&String> = keys.into_iter().collect();
    keys.sort_by_key(|key| match key.as_str() {
        "kind" => 0,
        "record" => 1,
        "id" => 2,
        "title" => 3,
        _ => 4,
    });
    keys
}

/// Whether a value is a non-empty list of records.
fn records(value: &Json) -> bool {
    value.as_array().is_some_and(|values| !values.is_empty() && values.iter().all(Json::is_object))
}

/// Whether a value says nothing: null, or an empty list, record or text.
fn empty(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::String(text) => text.is_empty(),
        Json::Array(values) => values.is_empty(),
        Json::Object(fields) => fields.values().all(empty),
        _ => false,
    }
}

/// A readiness, an item's or a thread's, as a phrase: `waiting for a (not done)`, `claimed by session/s`, and so on.
/// `None` for a readiness this build doesn't know, which then prints field by field.
fn readiness(fields: &serde_json::Map<String, Json>) -> Option<String> {
    let text = |key: &str| fields.get(key).map(compact).unwrap_or_default();
    // A thread's readiness lists the members that would be ready while it is held.
    let ready = match fields.get("items") {
        Some(items) if !empty(items) => format!("; ready: {}", compact(items)),
        _ => String::new(),
    };
    let phrase = match fields.get("readiness")?.as_str()? {
        "waiting" => {
            let waiting = fields.get("waiting")?.as_object()?;
            let each = waiting.iter().map(|(id, wait)| {
                let why = match wait["why"].as_str() {
                    Some("open") => "not done".to_owned(),
                    _ => compact(wait),
                };
                format!("{id} ({why})")
            });
            format!("waiting for {}", each.collect::<Vec<_>>().join(", "))
        },
        "ready" if ready.is_empty() => "ready".to_owned(),
        "ready" => format!("ready: {}", compact(&fields["items"])),
        "claimed" => format!("claimed by {}{ready}", text("holder")),
        "contested" => format!("contested by {}{ready}", text("holders")),
        "conflicted" => format!("conflicted: {}", text("fields")),
        "in-cycle" => "in a cycle".to_owned(),
        "nothing-ready" => "nothing ready".to_owned(),
        // The rest say all there is in their name.
        other if fields.len() == 1 => other.replace('-', " "),
        _ => return None,
    };
    Some(phrase)
}

/// A value on one line: text as it is, a list joined with commas, an embedded record by its `id`, a readiness as a
/// phrase, and another record's fields that say something as `name value`, its `readiness` first.
fn compact(value: &Json) -> String {
    match value {
        Json::Object(fields) if let Some(phrase) = readiness(fields) => phrase,
        Json::Object(fields) if fields.len() == 1 && fields.contains_key("conflict") => {
            format!("conflict {}", conflict_values(&fields["conflict"]))
        },
        // A conflict's value with how it is kept: the line to run, or why nothing keeps it.
        Json::Object(fields) if fields.contains_key("run") && fields.contains_key("why") => {
            let value = match &fields["value"] {
                Json::Null => "none".to_owned(),
                Json::String(text) => text.clone(),
                other => other.to_string(),
            };
            match (&fields["run"], &fields["why"]) {
                (Json::String(run), _) => format!("keep {value}: {run}"),
                (_, why) => format!("{value}: {}", compact(why)),
            }
        },
        Json::Null => String::new(),
        Json::Bool(yes) => (if *yes { "yes" } else { "no" }).to_owned(),
        Json::String(text) => text.clone(),
        Json::Number(number) => number.to_string(),
        // An embedded record reads as its id, with its one other field when it has only one.
        Json::Object(fields) if fields.get("id").is_some_and(Json::is_string) => {
            let id = compact(&fields["id"]);
            match (fields.len(), fields.iter().find(|(key, _)| *key != "id")) {
                (2, Some((_, other))) if !other.is_object() => format!("{id} ({})", compact(other)),
                _ => id,
            }
        },
        Json::Array(values) => {
            let separator =
                if values.iter().any(|value| value.is_object() && value.get("id").is_none()) {
                    "; "
                } else {
                    ", "
                };
            values.iter().map(compact).collect::<Vec<_>>().join(separator)
        },
        Json::Object(fields) => {
            let rest = fields
                .iter()
                .filter(|(key, value)| *key != "readiness" && !empty(value))
                .map(|(key, value)| format!("{key} {}", compact(value)))
                .collect::<Vec<_>>()
                .join(", ");
            match (fields.get("readiness"), rest.is_empty()) {
                (Some(readiness), true) => compact(readiness),
                (Some(readiness), false) => format!("{} ({rest})", compact(readiness)),
                (None, _) => rest,
            }
        },
    }
}

/// A cleared side is a choice, not a missing value to leave out.
fn conflict_values(values: &Json) -> String {
    values
        .as_array()
        .into_iter()
        .flatten()
        .map(|value| if value.is_null() { "none".to_owned() } else { compact(value) })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
