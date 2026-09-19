//! Agents' questions: ACP form elicitations as AHP input requests.
//!
//! An agent asks with `elicitation/create`, carrying a JSON schema of flat
//! fields. Each field becomes a question keyed by the field's name, so the
//! answers map back onto the same names. Claude's AskUserQuestion pairs every
//! select field `question_N` with a free-text "Other" field, marked in its
//! `_meta`; that field is folded into its question as free-form input rather
//! than asked on its own.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use agent_client_protocol::schema::v1::{
    CreateElicitationResponse, ElicitationAcceptAction, ElicitationAction, ElicitationContentValue,
};
use ahp_types::state::{
    ChatInputAnswer, ChatInputAnswerValue, ChatInputBooleanQuestion, ChatInputMultiSelectQuestion,
    ChatInputNumberQuestion, ChatInputOption, ChatInputQuestion, ChatInputRequest,
    ChatInputResponseKind, ChatInputSingleSelectQuestion, ChatInputTextQuestion,
};
use serde_json::{Map, Value};

use crate::agent::InputAnswer;

/// The `_meta` key that marks a field as the free-text answer to another.
const CUSTOM_ANSWER_META_KEY: &str = "_askUserQuestionCustomAnswer";

/// An elicitation as the chat asks it, and how to read its answers back.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub request: ChatInputRequest,
    /// Question id to the field that takes its free-form text.
    custom: HashMap<String, String>,
}

/// Turns an elicitation's `message` and requested JSON `schema` into an input
/// request with the id `id`. Fields of a kind the chat cannot ask are left
/// out.
pub fn form(id: &str, message: &str, schema: &Value) -> Form {
    let empty = Map::new();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    // The fields' order is lost on the way here; names like `question_10`
    // still sort after `question_2`.
    let mut names: Vec<&String> = properties.keys().collect();
    names.sort_by(|a, b| natural(a, b));

    let mut questions: Vec<ChatInputQuestion> = Vec::new();
    let mut custom = HashMap::new();
    let mut free_text: Vec<(&str, &str)> = Vec::new();
    for name in names {
        let field = &properties[name.as_str()];
        if let Some(target) = custom_answer_for(field) {
            free_text.push((name, target));
        }
        let required = required.contains(&name.as_str()).then_some(true);
        if let Some(question) = question(name, field, required) {
            questions.push(question);
        }
    }
    // Fold each "Other" field into the select question it answers; one that
    // answers no select question stays a text question of its own.
    for (field, target) in free_text {
        let folded = questions.iter_mut().any(|question| match question {
            ChatInputQuestion::SingleSelect(select) if select.id == target => {
                select.allow_freeform_input = Some(true);
                true
            }
            ChatInputQuestion::MultiSelect(select) if select.id == target => {
                select.allow_freeform_input = Some(true);
                true
            }
            _ => false,
        });
        if folded {
            questions.retain(|question| question_id(question) != Some(field));
            custom.insert(target.to_owned(), field.to_owned());
        }
    }

    Form {
        request: ChatInputRequest {
            id: id.to_owned(),
            message: (!message.is_empty()).then(|| message.to_owned()),
            url: None,
            questions: Some(questions),
            answers: None,
        },
        custom,
    }
}

/// The agent's answer: the fields' values under their own names for an
/// accepted form, or a decline or cancel. With no answer at all, cancelled.
pub fn respond(form: &Form, answer: Option<InputAnswer>) -> CreateElicitationResponse {
    let action = match answer {
        Some(InputAnswer {
            response: ChatInputResponseKind::Accept,
            answers,
        }) => ElicitationAction::Accept(
            ElicitationAcceptAction::new().content(content(form, &answers)),
        ),
        Some(InputAnswer {
            response: ChatInputResponseKind::Decline,
            ..
        }) => ElicitationAction::Decline,
        _ => ElicitationAction::Cancel,
    };
    CreateElicitationResponse::new(action)
}

/// The submitted `answers` as the form's field values.
pub fn content(
    form: &Form,
    answers: &HashMap<String, ChatInputAnswer>,
) -> BTreeMap<String, ElicitationContentValue> {
    let mut content = BTreeMap::new();
    for question in form.request.questions.iter().flatten() {
        let Some(id) = question_id(question) else {
            continue;
        };
        let custom = form.custom.get(id);
        let freeform = |values: &Option<Vec<String>>, content: &mut BTreeMap<_, _>| {
            let text = values
                .iter()
                .flatten()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(field) = custom.filter(|_| !text.is_empty()) {
                content.insert(field.clone(), ElicitationContentValue::String(text));
            }
        };
        let value = match answers.get(id) {
            Some(ChatInputAnswer::Submitted(answered)) => &answered.value,
            Some(ChatInputAnswer::Skipped(skipped)) => {
                freeform(&skipped.freeform_values, &mut content);
                continue;
            }
            _ => continue,
        };
        match (question, value) {
            (ChatInputQuestion::Text(_), ChatInputAnswerValue::Text(text)) => {
                content.insert(
                    id.to_owned(),
                    ElicitationContentValue::String(text.value.clone()),
                );
            }
            (ChatInputQuestion::Integer(_), ChatInputAnswerValue::Number(number)) => {
                content.insert(
                    id.to_owned(),
                    ElicitationContentValue::Integer(number.value.round() as i64),
                );
            }
            (ChatInputQuestion::Number(_), ChatInputAnswerValue::Number(number)) => {
                content.insert(id.to_owned(), ElicitationContentValue::Number(number.value));
            }
            (ChatInputQuestion::Boolean(_), ChatInputAnswerValue::Boolean(boolean)) => {
                content.insert(
                    id.to_owned(),
                    ElicitationContentValue::Boolean(boolean.value),
                );
            }
            (ChatInputQuestion::SingleSelect(select), ChatInputAnswerValue::Selected(selected)) => {
                let mut freeform_values = selected.freeform_values.clone().unwrap_or_default();
                if select
                    .options
                    .iter()
                    .any(|option| option.id == selected.value)
                {
                    content.insert(
                        id.to_owned(),
                        ElicitationContentValue::String(selected.value.clone()),
                    );
                } else if !selected.value.is_empty() {
                    // Not one of the options: text typed in their place.
                    freeform_values.insert(0, selected.value.clone());
                }
                freeform(&Some(freeform_values), &mut content);
            }
            (
                ChatInputQuestion::MultiSelect(select),
                ChatInputAnswerValue::SelectedMany(selected),
            ) => {
                let (picked, typed): (Vec<String>, Vec<String>) = selected
                    .value
                    .iter()
                    .filter(|value| !value.is_empty())
                    .cloned()
                    .partition(|value| select.options.iter().any(|option| option.id == *value));
                content.insert(id.to_owned(), ElicitationContentValue::StringArray(picked));
                let freeform_values = typed
                    .into_iter()
                    .chain(selected.freeform_values.clone().unwrap_or_default())
                    .collect();
                freeform(&Some(freeform_values), &mut content);
            }
            _ => {}
        }
    }
    content
}

/// Whether `answer` fits `question`: a value of the question's kind, or a
/// skip.
pub fn answer_fits(question: &ChatInputQuestion, answer: &ChatInputAnswer) -> bool {
    let value = match answer {
        ChatInputAnswer::Draft(answered) | ChatInputAnswer::Submitted(answered) => &answered.value,
        ChatInputAnswer::Skipped(_) => return true,
    };
    matches!(
        (question, value),
        (ChatInputQuestion::Text(_), ChatInputAnswerValue::Text(_))
            | (
                ChatInputQuestion::Number(_),
                ChatInputAnswerValue::Number(_)
            )
            | (
                ChatInputQuestion::Integer(_),
                ChatInputAnswerValue::Number(_)
            )
            | (
                ChatInputQuestion::Boolean(_),
                ChatInputAnswerValue::Boolean(_)
            )
            | (
                ChatInputQuestion::SingleSelect(_),
                ChatInputAnswerValue::Selected(_)
            )
            | (
                ChatInputQuestion::MultiSelect(_),
                ChatInputAnswerValue::SelectedMany(_)
            )
    )
}

/// The id of `question`, unless it is of a kind this host does not know.
pub fn question_id(question: &ChatInputQuestion) -> Option<&str> {
    match question {
        ChatInputQuestion::Text(q) => Some(&q.id),
        ChatInputQuestion::Number(q) | ChatInputQuestion::Integer(q) => Some(&q.id),
        ChatInputQuestion::Boolean(q) => Some(&q.id),
        ChatInputQuestion::SingleSelect(q) => Some(&q.id),
        ChatInputQuestion::MultiSelect(q) => Some(&q.id),
        ChatInputQuestion::Unknown(_) => None,
    }
}

/// Whether `question` must be answered for the request to be accepted.
pub fn is_required(question: &ChatInputQuestion) -> bool {
    let required = match question {
        ChatInputQuestion::Text(q) => q.required,
        ChatInputQuestion::Number(q) | ChatInputQuestion::Integer(q) => q.required,
        ChatInputQuestion::Boolean(q) => q.required,
        ChatInputQuestion::SingleSelect(q) => q.required,
        ChatInputQuestion::MultiSelect(q) => q.required,
        ChatInputQuestion::Unknown(_) => None,
    };
    required == Some(true)
}

/// The field another field is the free-text answer to, when it is one.
fn custom_answer_for(field: &Value) -> Option<&str> {
    let marker = field.get("_meta")?.get(CUSTOM_ANSWER_META_KEY)?;
    if marker.get("isCustomAnswer").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    marker.get("questionId")?.as_str()
}

fn question(name: &str, field: &Value, required: Option<bool>) -> Option<ChatInputQuestion> {
    let text = |key: &str| field.get(key).and_then(Value::as_str).map(str::to_owned);
    let title = text("title");
    let message = text("description")
        .or_else(|| title.clone())
        .unwrap_or_else(|| name.to_owned());
    let id = name.to_owned();
    let question = match field.get("type")?.as_str()? {
        "string" => match options(field) {
            Some(options) => ChatInputQuestion::SingleSelect(ChatInputSingleSelectQuestion {
                id,
                title,
                message,
                required,
                options: recommend(options, field.get("default")),
                allow_freeform_input: None,
            }),
            None => ChatInputQuestion::Text(ChatInputTextQuestion {
                id,
                title,
                message,
                required,
                format: text("format"),
                min: field.get("minLength").and_then(Value::as_i64),
                max: field.get("maxLength").and_then(Value::as_i64),
                default_value: text("default"),
            }),
        },
        kind @ ("number" | "integer") => {
            let number = ChatInputNumberQuestion {
                id,
                title,
                message,
                required,
                min: field.get("minimum").and_then(Value::as_f64),
                max: field.get("maximum").and_then(Value::as_f64),
                default_value: field.get("default").and_then(Value::as_f64),
            };
            if kind == "integer" {
                ChatInputQuestion::Integer(number)
            } else {
                ChatInputQuestion::Number(number)
            }
        }
        "boolean" => ChatInputQuestion::Boolean(ChatInputBooleanQuestion {
            id,
            title,
            message,
            required,
            default_value: field.get("default").and_then(Value::as_bool),
        }),
        "array" => ChatInputQuestion::MultiSelect(ChatInputMultiSelectQuestion {
            id,
            title,
            message,
            required,
            options: recommend(options(field.get("items")?)?, field.get("default")),
            allow_freeform_input: None,
            min: field.get("minItems").and_then(Value::as_i64),
            max: field.get("maxItems").and_then(Value::as_i64),
        }),
        _ => return None,
    };
    Some(question)
}

/// A select field's options: titled (`oneOf` or `anyOf` of `const`s) or plain
/// (`enum`).
fn options(field: &Value) -> Option<Vec<ChatInputOption>> {
    let titled = field
        .get("oneOf")
        .or_else(|| field.get("anyOf"))
        .and_then(Value::as_array);
    if let Some(titled) = titled {
        let options = titled
            .iter()
            .filter_map(|option| {
                let id = option.get("const")?.as_str()?.to_owned();
                let label = option
                    .get("title")
                    .and_then(Value::as_str)
                    .map_or_else(|| id.clone(), str::to_owned);
                Some(ChatInputOption {
                    id,
                    label,
                    description: option
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    recommended: None,
                })
            })
            .collect();
        return Some(options);
    }
    let plain = field.get("enum")?.as_array()?;
    Some(
        plain
            .iter()
            .filter_map(Value::as_str)
            .map(|value| ChatInputOption {
                id: value.to_owned(),
                label: value.to_owned(),
                description: None,
                recommended: None,
            })
            .collect(),
    )
}

/// Marks the options a field's `default` names as recommended.
fn recommend(mut options: Vec<ChatInputOption>, default: Option<&Value>) -> Vec<ChatInputOption> {
    let defaults: Vec<&str> = match default {
        Some(Value::String(value)) => vec![value.as_str()],
        Some(Value::Array(values)) => values.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    for option in &mut options {
        if defaults.contains(&option.id.as_str()) {
            option.recommended = Some(true);
        }
    }
    options
}

/// Compares names with their runs of digits taken as numbers.
fn natural(a: &str, b: &str) -> Ordering {
    fn runs(s: &str) -> Vec<(bool, &str)> {
        let mut runs = Vec::new();
        let mut start = 0;
        let bytes = s.as_bytes();
        for i in 1..=bytes.len() {
            if i == bytes.len() || bytes[i].is_ascii_digit() != bytes[start].is_ascii_digit() {
                runs.push((bytes[start].is_ascii_digit(), &s[start..i]));
                start = i;
            }
        }
        runs
    }
    let (ra, rb) = (runs(a), runs(b));
    for (x, y) in ra.iter().zip(&rb) {
        let order = match (x, y) {
            ((true, x), (true, y)) => {
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                x.len().cmp(&y.len()).then_with(|| x.cmp(y))
            }
            ((_, x), (_, y)) => x.cmp(y),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    ra.len().cmp(&rb.len()).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use ahp_types::state::{
        ChatInputAnswered, ChatInputBooleanAnswerValue, ChatInputNumberAnswerValue,
        ChatInputSelectedAnswerValue, ChatInputSelectedManyAnswerValue, ChatInputSkipped,
        ChatInputTextAnswerValue,
    };
    use serde_json::json;

    use super::*;

    /// What claude-agent-acp sends for AskUserQuestion with two questions: a
    /// single select and a multi select, each with its "Other" field.
    fn ask_user_question() -> Value {
        let other = |question: &str| {
            json!({
                "type": "string",
                "title": "Other",
                "description": "Type your own answer (optional).",
                "_meta": { "_askUserQuestionCustomAnswer": { "questionId": question, "isCustomAnswer": true } },
            })
        };
        json!({
            "type": "object",
            "properties": {
                "question_0": {
                    "type": "string",
                    "title": "Database",
                    "description": "Which database should we use?",
                    "oneOf": [
                        { "const": "Postgres", "title": "Postgres", "description": "Relational" },
                        { "const": "SQLite", "title": "SQLite" },
                    ],
                },
                "question_0_custom": other("question_0"),
                "question_1": {
                    "type": "array",
                    "title": "Features",
                    "description": "Which features?",
                    "items": { "anyOf": [
                        { "const": "Auth", "title": "Auth" },
                        { "const": "Search", "title": "Search" },
                    ] },
                },
                "question_1_custom": other("question_1"),
            },
        })
    }

    fn submitted(value: ChatInputAnswerValue) -> ChatInputAnswer {
        ChatInputAnswer::Submitted(ChatInputAnswered { value })
    }

    fn selected(value: &str, freeform: &[&str]) -> ChatInputAnswer {
        submitted(ChatInputAnswerValue::Selected(
            ChatInputSelectedAnswerValue {
                value: value.into(),
                freeform_values: (!freeform.is_empty())
                    .then(|| freeform.iter().map(|s| s.to_string()).collect()),
            },
        ))
    }

    fn content_json(form: &Form, answers: &[(&str, ChatInputAnswer)]) -> Value {
        let answers = answers
            .iter()
            .map(|(id, answer)| (id.to_string(), answer.clone()))
            .collect();
        serde_json::to_value(content(form, &answers)).unwrap()
    }

    #[test]
    fn ask_user_question_becomes_select_questions_with_free_text() {
        let form = form(
            "r1",
            "Please answer the following questions.",
            &ask_user_question(),
        );
        let request = serde_json::to_value(&form.request).unwrap();
        assert_eq!(
            request,
            json!({
                "id": "r1",
                "message": "Please answer the following questions.",
                "questions": [
                    {
                        "kind": "single-select",
                        "id": "question_0",
                        "title": "Database",
                        "message": "Which database should we use?",
                        "options": [
                            { "id": "Postgres", "label": "Postgres", "description": "Relational" },
                            { "id": "SQLite", "label": "SQLite" },
                        ],
                        "allowFreeformInput": true,
                    },
                    {
                        "kind": "multi-select",
                        "id": "question_1",
                        "title": "Features",
                        "message": "Which features?",
                        "options": [
                            { "id": "Auth", "label": "Auth" },
                            { "id": "Search", "label": "Search" },
                        ],
                        "allowFreeformInput": true,
                    },
                ],
            })
        );
    }

    #[test]
    fn answers_come_back_under_the_field_names() {
        let form = form("r1", "", &ask_user_question());
        let many = submitted(ChatInputAnswerValue::SelectedMany(
            ChatInputSelectedManyAnswerValue {
                value: vec!["Auth".into()],
                freeform_values: Some(vec!["Billing".into()]),
            },
        ));
        assert_eq!(
            content_json(
                &form,
                &[
                    ("question_0", selected("SQLite", &[])),
                    ("question_1", many)
                ]
            ),
            json!({
                "question_0": "SQLite",
                "question_1": ["Auth"],
                "question_1_custom": "Billing",
            })
        );
    }

    #[test]
    fn free_text_in_place_of_an_option_goes_to_the_other_field() {
        let form = form("r1", "", &ask_user_question());
        assert_eq!(
            content_json(&form, &[("question_0", selected("", &["MariaDB"]))]),
            json!({ "question_0_custom": "MariaDB" })
        );
        assert_eq!(
            content_json(&form, &[("question_0", selected("DuckDB", &[]))]),
            json!({ "question_0_custom": "DuckDB" })
        );
        let skipped = ChatInputAnswer::Skipped(ChatInputSkipped {
            freeform_values: Some(vec!["none of these".into()]),
        });
        assert_eq!(
            content_json(&form, &[("question_0", skipped)]),
            json!({ "question_0_custom": "none of these" })
        );
    }

    #[test]
    fn drafts_are_not_answers() {
        let form = form("r1", "", &ask_user_question());
        let draft = ChatInputAnswer::Draft(ChatInputAnswered {
            value: ChatInputAnswerValue::Selected(ChatInputSelectedAnswerValue {
                value: "SQLite".into(),
                freeform_values: None,
            }),
        });
        assert_eq!(content_json(&form, &[("question_0", draft)]), json!({}));
    }

    #[test]
    fn generic_fields_keep_their_kinds() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "title": "Name", "format": "email", "minLength": 3 },
                "count": { "type": "integer", "description": "How many?", "minimum": 1, "default": 2 },
                "ratio": { "type": "number" },
                "ok": { "type": "boolean", "default": true },
                "color": { "type": "string", "enum": ["red", "blue"], "default": "blue" },
                "blob": { "type": "object" },
            },
            "required": ["name"],
        });
        let form = form("r2", "Fill this in", &schema);
        let questions = serde_json::to_value(&form.request.questions).unwrap();
        let kinds: Vec<(&str, &str)> = questions
            .as_array()
            .unwrap()
            .iter()
            .map(|q| (q["id"].as_str().unwrap(), q["kind"].as_str().unwrap()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("color", "single-select"),
                ("count", "integer"),
                ("name", "text"),
                ("ok", "boolean"),
                ("ratio", "number"),
            ]
        );
        assert_eq!(questions[0]["options"][1]["recommended"], json!(true));
        assert_eq!(questions[1]["message"], json!("How many?"));
        assert_eq!(questions[1]["defaultValue"], json!(2.0));
        assert_eq!(questions[2]["required"], json!(true));
        assert_eq!(questions[2]["format"], json!("email"));
        assert_eq!(questions[2]["min"], json!(3));

        let number = |value| {
            submitted(ChatInputAnswerValue::Number(ChatInputNumberAnswerValue {
                value,
            }))
        };
        let answers = [
            (
                "name",
                submitted(ChatInputAnswerValue::Text(ChatInputTextAnswerValue {
                    value: "a@b.c".into(),
                })),
            ),
            ("count", number(3.0)),
            ("ratio", number(0.5)),
            (
                "ok",
                submitted(ChatInputAnswerValue::Boolean(ChatInputBooleanAnswerValue {
                    value: false,
                })),
            ),
            ("color", selected("red", &[])),
        ];
        assert_eq!(
            content_json(&form, &answers),
            json!({ "name": "a@b.c", "count": 3, "ratio": 0.5, "ok": false, "color": "red" })
        );
    }

    #[test]
    fn responses_map_to_elicitation_actions() {
        let form = form("r1", "", &ask_user_question());
        let action = |answer| serde_json::to_value(respond(&form, answer)).unwrap();
        assert_eq!(
            action(Some(InputAnswer::decline())),
            json!({ "action": "decline" })
        );
        assert_eq!(action(None), json!({ "action": "cancel" }));
        let accept = InputAnswer {
            response: ChatInputResponseKind::Accept,
            answers: HashMap::from([("question_0".to_owned(), selected("Postgres", &[]))]),
        };
        assert_eq!(
            action(Some(accept)),
            json!({ "action": "accept", "content": { "question_0": "Postgres" } })
        );
    }

    #[test]
    fn numbered_fields_sort_by_number() {
        let mut names = vec![
            "question_10",
            "question_2",
            "question_1_custom",
            "question_1",
        ];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(
            names,
            [
                "question_1",
                "question_1_custom",
                "question_2",
                "question_10"
            ]
        );
    }
}
