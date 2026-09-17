//! Questions an agent asks the person: input requests.
//!
//! An agent that needs something from the person — a choice between
//! approaches, a value an MCP server elicits, a link to review — puts an input
//! request in its turn (`InputRequestResponsePart`). It stays in the turn for
//! good: open while it waits, then answered, declined or dismissed. Every
//! client watching the chat sees the same request and the same draft answers,
//! so one question can be answered here and the next one somewhere else.
//!
//! This module reads those requests and works out what the launcher offers
//! for them: the rows under the field, what the log says, and the answers
//! each choice produces. The launcher asks one question at a time. What the
//! connection sends is decided in [`crate::ask`].

use std::collections::HashMap;

use ahp_types::state::{
    ChatInputAnswer, ChatInputAnswerValue, ChatInputAnswered, ChatInputBooleanAnswerValue,
    ChatInputNumberAnswerValue, ChatInputOption, ChatInputQuestion, ChatInputResponseKind,
    ChatInputSelectedAnswerValue, ChatInputSelectedManyAnswerValue, ChatInputSkipped,
    ChatInputTextAnswerValue, InputRequestResponsePart,
};

use crate::log::Style;

/// Where an input request stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Waiting for an answer, in the turn that is running.
    Open,
    /// The turn ended before anyone answered.
    Unanswered,
    /// Someone answered it, here or elsewhere.
    Responded(ChatInputResponseKind),
}

/// A request for input, as the launcher shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct InputRequest {
    pub id: String,
    pub message: Option<String>,
    pub url: Option<String>,
    /// The questions the launcher knows how to ask, in order. A kind from a
    /// newer protocol is left out: there is no way to answer it here.
    pub fields: Vec<Field>,
    /// Draft, submitted and skipped answers, by question id.
    pub answers: HashMap<String, ChatInputAnswer>,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub id: String,
    pub title: Option<String>,
    pub message: String,
    pub required: bool,
    pub kind: FieldKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Text {
        default: Option<String>,
        /// Length bounds, in characters.
        min: Option<i64>,
        max: Option<i64>,
    },
    Number {
        integer: bool,
        min: Option<f64>,
        max: Option<f64>,
        default: Option<f64>,
    },
    Boolean {
        default: Option<bool>,
    },
    Single {
        options: Vec<ChatInputOption>,
        /// The person may type an answer of their own instead.
        freeform: bool,
    },
    Multi {
        options: Vec<ChatInputOption>,
        /// The person may type answers of their own as well.
        freeform: bool,
        /// Bounds on how many are picked.
        min: Option<i64>,
        max: Option<i64>,
    },
}

/// A row under the field while a request is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// An option of a single- or multi-select question.
    Option(usize),
    Yes,
    No,
    /// Take what is typed, or what is picked, and move on.
    Continue,
    /// Skip a question that is not required.
    Skip,
    OpenLink,
    /// Send the answers: every question is answered, or there were none.
    Send,
    Decline,
}

/// Why an answer was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    Empty,
    NotNumber,
    NotInteger,
    Below(String),
    Above(String),
    TooShort(i64),
    TooLong(i64),
    PickAtLeast(i64),
    PickAtMost(i64),
}

impl Invalid {
    pub fn text(&self) -> String {
        match self {
            Invalid::Empty => otto_kit::t_owned!("launcher-input-error-empty"),
            Invalid::NotNumber => otto_kit::t_owned!("launcher-input-error-number"),
            Invalid::NotInteger => otto_kit::t_owned!("launcher-input-error-integer"),
            Invalid::Below(min) => {
                otto_kit::t_owned!("launcher-input-error-min", min = min.as_str())
            }
            Invalid::Above(max) => {
                otto_kit::t_owned!("launcher-input-error-max", max = max.as_str())
            }
            Invalid::TooShort(min) => {
                otto_kit::t_owned!("launcher-input-error-short", min = min.to_string())
            }
            Invalid::TooLong(max) => {
                otto_kit::t_owned!("launcher-input-error-long", max = max.to_string())
            }
            Invalid::PickAtLeast(min) => {
                otto_kit::t_owned!("launcher-input-error-pick-min", min = min.to_string())
            }
            Invalid::PickAtMost(max) => {
                otto_kit::t_owned!("launcher-input-error-pick-max", max = max.to_string())
            }
        }
    }
}

impl Field {
    fn from_question(question: &ChatInputQuestion) -> Option<Self> {
        let field = |id: &str, title: &Option<String>, message: &str, required, kind| Field {
            id: id.to_string(),
            title: title.clone().filter(|title| !title.trim().is_empty()),
            message: message.to_string(),
            required: required == Some(true),
            kind,
        };
        Some(match question {
            ChatInputQuestion::Text(q) => field(
                &q.id,
                &q.title,
                &q.message,
                q.required,
                FieldKind::Text {
                    default: q.default_value.clone(),
                    min: q.min,
                    max: q.max,
                },
            ),
            ChatInputQuestion::Number(q) | ChatInputQuestion::Integer(q) => field(
                &q.id,
                &q.title,
                &q.message,
                q.required,
                FieldKind::Number {
                    integer: matches!(question, ChatInputQuestion::Integer(_)),
                    min: q.min,
                    max: q.max,
                    default: q.default_value,
                },
            ),
            ChatInputQuestion::Boolean(q) => field(
                &q.id,
                &q.title,
                &q.message,
                q.required,
                FieldKind::Boolean {
                    default: q.default_value,
                },
            ),
            ChatInputQuestion::SingleSelect(q) => field(
                &q.id,
                &q.title,
                &q.message,
                q.required,
                FieldKind::Single {
                    options: q.options.clone(),
                    freeform: q.allow_freeform_input == Some(true),
                },
            ),
            ChatInputQuestion::MultiSelect(q) => field(
                &q.id,
                &q.title,
                &q.message,
                q.required,
                FieldKind::Multi {
                    options: q.options.clone(),
                    freeform: q.allow_freeform_input == Some(true),
                    min: q.min,
                    max: q.max,
                },
            ),
            ChatInputQuestion::Unknown(_) => return None,
        })
    }

    /// What the question is called in a one-line summary.
    pub fn label(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.message)
    }

    /// Whether typing into the field answers this question.
    pub fn takes_text(&self) -> bool {
        match &self.kind {
            FieldKind::Text { .. } | FieldKind::Number { .. } => true,
            FieldKind::Single { freeform, .. } | FieldKind::Multi { freeform, .. } => *freeform,
            FieldKind::Boolean { .. } => false,
        }
    }

    /// The option ids and typed answers picked so far for a multi-select
    /// question, from `answer`.
    fn picked(answer: Option<&ChatInputAnswer>) -> (Vec<String>, Vec<String>) {
        match answer {
            Some(ChatInputAnswer::Draft(answered) | ChatInputAnswer::Submitted(answered)) => {
                match &answered.value {
                    ChatInputAnswerValue::SelectedMany(many) => (
                        many.value.clone(),
                        many.freeform_values.clone().unwrap_or_default(),
                    ),
                    ChatInputAnswerValue::Selected(one) => (
                        vec![one.value.clone()],
                        one.freeform_values.clone().unwrap_or_default(),
                    ),
                    _ => Default::default(),
                }
            }
            _ => Default::default(),
        }
    }

    /// Checks `typed` against the question and makes it an answer value.
    pub fn parse(&self, typed: &str) -> Result<ChatInputAnswerValue, Invalid> {
        let typed = typed.trim();
        match &self.kind {
            FieldKind::Text { min, max, .. } => {
                if typed.is_empty() {
                    return Err(Invalid::Empty);
                }
                let length = typed.chars().count() as i64;
                if let Some(min) = min.filter(|min| length < *min) {
                    return Err(Invalid::TooShort(min));
                }
                if let Some(max) = max.filter(|max| length > *max) {
                    return Err(Invalid::TooLong(max));
                }
                Ok(ChatInputAnswerValue::Text(ChatInputTextAnswerValue {
                    value: typed.to_string(),
                }))
            }
            FieldKind::Number {
                integer, min, max, ..
            } => {
                if typed.is_empty() {
                    return Err(Invalid::Empty);
                }
                let value = parse_number(typed).ok_or(Invalid::NotNumber)?;
                if *integer && value.fract() != 0.0 {
                    return Err(Invalid::NotInteger);
                }
                if let Some(min) = min.filter(|min| value < *min) {
                    return Err(Invalid::Below(format_number(min)));
                }
                if let Some(max) = max.filter(|max| value > *max) {
                    return Err(Invalid::Above(format_number(max)));
                }
                Ok(ChatInputAnswerValue::Number(ChatInputNumberAnswerValue {
                    value,
                }))
            }
            FieldKind::Single { freeform: true, .. } if !typed.is_empty() => {
                // The typed answer stands in for an option. It goes in both
                // places, so a consumer that only reads `value` still gets it.
                Ok(ChatInputAnswerValue::Selected(
                    ChatInputSelectedAnswerValue {
                        value: typed.to_string(),
                        freeform_values: Some(vec![typed.to_string()]),
                    },
                ))
            }
            _ => Err(Invalid::Empty),
        }
    }

    /// A multi-select answer made of `picked` option ids and `typed` answers,
    /// checked against the question's bounds.
    fn many(
        &self,
        picked: Vec<String>,
        typed: Vec<String>,
    ) -> Result<ChatInputAnswerValue, Invalid> {
        if let FieldKind::Multi { min, max, .. } = &self.kind {
            let count = (picked.len() + typed.len()) as i64;
            let floor = min.unwrap_or(0).max(i64::from(self.required));
            if count < floor {
                return Err(Invalid::PickAtLeast(floor));
            }
            if let Some(max) = max.filter(|max| count > *max) {
                return Err(Invalid::PickAtMost(max));
            }
        }
        Ok(ChatInputAnswerValue::SelectedMany(
            ChatInputSelectedManyAnswerValue {
                value: picked,
                freeform_values: (!typed.is_empty()).then_some(typed),
            },
        ))
    }
}

/// A number as a person types it: a comma works as the decimal point.
fn parse_number(typed: &str) -> Option<f64> {
    let normal = if typed.contains('.') {
        typed.to_string()
    } else {
        typed.replacen(',', ".", 1)
    };
    normal.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// A number without a trailing `.0` when it is whole.
pub fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

/// Whether `answer` settles its question: submitted or skipped.
pub fn settled(answer: Option<&ChatInputAnswer>) -> bool {
    matches!(
        answer,
        Some(ChatInputAnswer::Submitted(_) | ChatInputAnswer::Skipped(_))
    )
}

pub fn submitted(value: ChatInputAnswerValue) -> ChatInputAnswer {
    ChatInputAnswer::Submitted(ChatInputAnswered { value })
}

pub fn draft(value: ChatInputAnswerValue) -> ChatInputAnswer {
    ChatInputAnswer::Draft(ChatInputAnswered { value })
}

pub fn skipped() -> ChatInputAnswer {
    ChatInputAnswer::Skipped(ChatInputSkipped::default())
}

/// What a change to the request does, for the connection to send.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// A question's answer changed: `chat/inputAnswerChanged`.
    Answer {
        question_id: String,
        answer: ChatInputAnswer,
    },
    /// The request is done: `chat/inputCompleted`.
    Complete {
        response: ChatInputResponseKind,
        answers: Option<HashMap<String, ChatInputAnswer>>,
    },
}

/// What choosing a row, or typing an answer, came to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    /// What to send, in order.
    pub changes: Vec<Change>,
    /// The question to show next, when that is not simply the first one left
    /// unanswered.
    pub next: Option<usize>,
    /// The typed text went into the answer, and the field can empty.
    pub took_text: bool,
    /// The link to open.
    pub open: Option<String>,
    pub invalid: Option<Invalid>,
}

impl InputRequest {
    /// The request in `part`. `active` says whether its turn is still running,
    /// which is the only place a request can still be answered.
    pub fn from_part(part: &InputRequestResponsePart, active: bool) -> Self {
        let request = &part.request;
        Self {
            id: request.id.clone(),
            message: request.message.clone().filter(|m| !m.trim().is_empty()),
            url: request.url.clone(),
            fields: request
                .questions
                .iter()
                .flatten()
                .filter_map(Field::from_question)
                .collect(),
            answers: request.answers.clone().unwrap_or_default(),
            outcome: match part.response {
                Some(response) => Outcome::Responded(response),
                None if active => Outcome::Open,
                None => Outcome::Unanswered,
            },
        }
    }

    pub fn is_open(&self) -> bool {
        self.outcome == Outcome::Open
    }

    /// The first question nobody has answered or skipped yet.
    pub fn first_unsettled(&self) -> Option<usize> {
        self.fields
            .iter()
            .position(|field| !settled(self.answers.get(&field.id)))
    }

    /// The link, when it is one a browser should open. Anything but a web link
    /// is shown and not opened: a handler for an arbitrary scheme is not
    /// something an agent gets to launch.
    pub fn web_link(&self) -> Option<&str> {
        self.url
            .as_deref()
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
    }

    /// The rows for question `current`, or for sending the request once no
    /// question is left.
    pub fn rows(&self, current: Option<usize>) -> Vec<Row> {
        let mut rows = Vec::new();
        match current.and_then(|index| self.fields.get(index)) {
            Some(field) => {
                match &field.kind {
                    FieldKind::Single { options, .. } => {
                        rows.extend((0..options.len()).map(Row::Option))
                    }
                    FieldKind::Multi { options, .. } => {
                        rows.extend((0..options.len()).map(Row::Option));
                        rows.push(Row::Continue);
                    }
                    FieldKind::Boolean { .. } => rows.extend([Row::Yes, Row::No]),
                    FieldKind::Text { .. } | FieldKind::Number { .. } => rows.push(Row::Continue),
                }
                if !field.required {
                    rows.push(Row::Skip);
                }
            }
            None => {
                if self.web_link().is_some() {
                    rows.push(Row::OpenLink);
                }
                rows.push(Row::Send);
            }
        }
        rows.push(Row::Decline);
        rows
    }

    /// The row to start from on question `current`: the answer already given
    /// or drafted, else the suggested one, else the first.
    pub fn default_row(&self, current: Option<usize>) -> usize {
        let rows = self.rows(current);
        let position = |wanted: Row| rows.iter().position(|row| *row == wanted);
        let Some(field) = current.and_then(|index| self.fields.get(index)) else {
            return position(Row::Send).unwrap_or(0);
        };
        let answer = self.answers.get(&field.id);
        let value = match answer {
            Some(ChatInputAnswer::Draft(a) | ChatInputAnswer::Submitted(a)) => Some(&a.value),
            _ => None,
        };
        let found = match (&field.kind, value) {
            (FieldKind::Single { options, .. }, value) => {
                let chosen = match value {
                    Some(ChatInputAnswerValue::Selected(one)) => {
                        options.iter().position(|o| o.id == one.value)
                    }
                    _ => None,
                };
                chosen
                    .or_else(|| options.iter().position(|o| o.recommended == Some(true)))
                    .and_then(|index| position(Row::Option(index)))
            }
            (FieldKind::Boolean { default }, value) => {
                let yes = match value {
                    Some(ChatInputAnswerValue::Boolean(b)) => Some(b.value),
                    _ => *default,
                };
                yes.and_then(|yes| position(if yes { Row::Yes } else { Row::No }))
            }
            _ => None,
        };
        found.unwrap_or(0)
    }

    /// What the field starts with on question `current`: the text already
    /// given or drafted, else the question's default.
    pub fn prefill(&self, current: Option<usize>) -> Option<String> {
        let field = self.fields.get(current?)?;
        let value = match self.answers.get(&field.id) {
            Some(ChatInputAnswer::Draft(a) | ChatInputAnswer::Submitted(a)) => Some(&a.value),
            _ => None,
        };
        match (&field.kind, value) {
            (FieldKind::Text { .. }, Some(ChatInputAnswerValue::Text(text))) => {
                Some(text.value.clone())
            }
            (FieldKind::Number { .. }, Some(ChatInputAnswerValue::Number(number))) => {
                Some(format_number(number.value))
            }
            (FieldKind::Text { default, .. }, _) => default.clone(),
            (FieldKind::Number { default, .. }, _) => default.map(format_number),
            _ => None,
        }
    }

    /// What the empty field says on question `current`.
    pub fn placeholder(&self, current: Option<usize>) -> Option<&'static str> {
        let field = self.fields.get(current?)?;
        Some(match &field.kind {
            FieldKind::Text { .. } => otto_kit::t!("launcher-input-type-answer"),
            FieldKind::Number { .. } => otto_kit::t!("launcher-input-type-number"),
            FieldKind::Single { freeform: true, .. } | FieldKind::Multi { freeform: true, .. } => {
                otto_kit::t!("launcher-input-type-other")
            }
            _ => return None,
        })
    }

    /// A row's title and subtitle.
    pub fn row_text(&self, row: Row, current: Option<usize>) -> (String, Option<String>) {
        let field = current.and_then(|index| self.fields.get(index));
        match row {
            Row::Option(index) => {
                let Some(field) = field else {
                    return (String::new(), None);
                };
                let (options, multi) = match &field.kind {
                    FieldKind::Single { options, .. } => (options, false),
                    FieldKind::Multi { options, .. } => (options, true),
                    _ => return (String::new(), None),
                };
                let Some(option) = options.get(index) else {
                    return (String::new(), None);
                };
                let title = if multi {
                    let (picked, _) = Field::picked(self.answers.get(&field.id));
                    let mark = if picked.contains(&option.id) {
                        '☑'
                    } else {
                        '☐'
                    };
                    format!("{mark} {}", option.label)
                } else {
                    option.label.clone()
                };
                let suggested = (option.recommended == Some(true))
                    .then(|| otto_kit::t_owned!("launcher-input-suggested"));
                let subtitle = match (option.description.clone(), suggested) {
                    (Some(description), Some(suggested)) => {
                        Some(format!("{suggested} · {description}"))
                    }
                    (description, suggested) => description.or(suggested),
                };
                (title, subtitle)
            }
            Row::Yes => (otto_kit::t_owned!("launcher-input-yes"), None),
            Row::No => (otto_kit::t_owned!("launcher-input-no"), None),
            Row::Continue => (otto_kit::t_owned!("launcher-input-continue"), None),
            Row::Skip => (otto_kit::t_owned!("launcher-input-skip"), None),
            Row::OpenLink => (
                otto_kit::t_owned!("launcher-input-open-link"),
                self.url.clone(),
            ),
            Row::Send => (
                if self.fields.is_empty() {
                    otto_kit::t_owned!("launcher-input-done")
                } else {
                    otto_kit::t_owned!("launcher-input-send")
                },
                None,
            ),
            Row::Decline => (otto_kit::t_owned!("launcher-input-decline"), None),
        }
    }

    /// Choose `row` on question `current`, with `typed` in the field.
    pub fn choose(&self, row: Row, current: Option<usize>, typed: &str) -> Step {
        let field = current.and_then(|index| self.fields.get(index));
        match (row, field) {
            (Row::Decline, _) => Step {
                changes: vec![Change::Complete {
                    response: ChatInputResponseKind::Decline,
                    answers: None,
                }],
                ..Step::default()
            },
            (Row::OpenLink, _) => Step {
                open: self.web_link().map(str::to_string),
                ..Step::default()
            },
            (Row::Send, _) => self.complete(Vec::new()),
            (Row::Skip, Some(field)) => self.settle(current, field, skipped(), false),
            (Row::Yes | Row::No, Some(field)) => {
                let value = ChatInputAnswerValue::Boolean(ChatInputBooleanAnswerValue {
                    value: row == Row::Yes,
                });
                self.settle(current, field, submitted(value), false)
            }
            (Row::Option(index), Some(field)) => match &field.kind {
                FieldKind::Single { options, .. } => {
                    let Some(option) = options.get(index) else {
                        return Step::default();
                    };
                    let value = ChatInputAnswerValue::Selected(ChatInputSelectedAnswerValue {
                        value: option.id.clone(),
                        freeform_values: None,
                    });
                    self.settle(current, field, submitted(value), false)
                }
                FieldKind::Multi { options, .. } => {
                    let Some(option) = options.get(index) else {
                        return Step::default();
                    };
                    // Picking toggles, and the draft is shared as it changes.
                    let (mut picked, typed) = Field::picked(self.answers.get(&field.id));
                    match picked.iter().position(|id| *id == option.id) {
                        Some(at) => {
                            picked.remove(at);
                        }
                        None => picked.push(option.id.clone()),
                    }
                    let value =
                        ChatInputAnswerValue::SelectedMany(ChatInputSelectedManyAnswerValue {
                            value: picked,
                            freeform_values: (!typed.is_empty()).then_some(typed),
                        });
                    Step {
                        changes: vec![Change::Answer {
                            question_id: field.id.clone(),
                            answer: draft(value),
                        }],
                        next: current,
                        ..Step::default()
                    }
                }
                _ => Step::default(),
            },
            (Row::Continue, Some(field)) => match &field.kind {
                FieldKind::Multi { .. } => {
                    let (picked, mut extra) = Field::picked(self.answers.get(&field.id));
                    let typed = typed.trim();
                    let took_text = !typed.is_empty();
                    if took_text {
                        extra.push(typed.to_string());
                    }
                    match field.many(picked, extra) {
                        Ok(value) => self.settle(current, field, submitted(value), took_text),
                        Err(invalid) => Step {
                            invalid: Some(invalid),
                            next: current,
                            ..Step::default()
                        },
                    }
                }
                _ => self.answer_text(current, typed),
            },
            _ => Step::default(),
        }
    }

    /// Answer question `current` with what is typed. For a multi-select
    /// question, a typed answer joins the picked ones rather than moving on.
    pub fn answer_text(&self, current: Option<usize>, typed: &str) -> Step {
        let Some(field) = current.and_then(|index| self.fields.get(index)) else {
            return Step::default();
        };
        if let FieldKind::Multi { .. } = field.kind {
            let typed = typed.trim();
            if typed.is_empty() {
                return Step::default();
            }
            let (picked, mut extra) = Field::picked(self.answers.get(&field.id));
            extra.push(typed.to_string());
            return Step {
                changes: vec![Change::Answer {
                    question_id: field.id.clone(),
                    answer: draft(ChatInputAnswerValue::SelectedMany(
                        ChatInputSelectedManyAnswerValue {
                            value: picked,
                            freeform_values: Some(extra),
                        },
                    )),
                }],
                next: current,
                took_text: true,
                ..Step::default()
            };
        }
        match field.parse(typed) {
            Ok(value) => self.settle(current, field, submitted(value), true),
            // Nothing typed on a question that can go without is a skip.
            Err(Invalid::Empty) if !field.required => self.settle(current, field, skipped(), false),
            Err(invalid) => Step {
                invalid: Some(invalid),
                next: current,
                ..Step::default()
            },
        }
    }

    /// `answer` settles question `current`: send it, then move to the next
    /// question, or send the lot when this was the last one.
    fn settle(
        &self,
        current: Option<usize>,
        field: &Field,
        answer: ChatInputAnswer,
        took_text: bool,
    ) -> Step {
        let change = Change::Answer {
            question_id: field.id.clone(),
            answer: answer.clone(),
        };
        let index = current.unwrap_or(0);
        if index + 1 < self.fields.len() {
            return Step {
                changes: vec![change],
                next: Some(index + 1),
                took_text,
                ..Step::default()
            };
        }
        let mut step = self.complete(vec![(field.id.clone(), answer)]);
        if let Some(invalid) = step.invalid.take() {
            // A required question further up is still open: go there.
            return Step {
                changes: vec![change],
                next: self.first_unsettled().filter(|at| *at != index),
                took_text,
                invalid: Some(invalid),
                ..Step::default()
            };
        }
        step.changes.insert(0, change);
        step.took_text = took_text;
        step
    }

    /// Accept the request with its answers so far, plus `extra`. Refused while
    /// a required question has no submitted answer.
    fn complete(&self, extra: Vec<(String, ChatInputAnswer)>) -> Step {
        let mut answers: HashMap<String, ChatInputAnswer> = self
            .answers
            .iter()
            .filter(|(_, answer)| settled(Some(answer)))
            .map(|(id, answer)| (id.clone(), answer.clone()))
            .collect();
        answers.extend(extra);
        let missing = self.fields.iter().any(|field| {
            field.required && !matches!(answers.get(&field.id), Some(ChatInputAnswer::Submitted(_)))
        });
        if missing {
            return Step {
                invalid: Some(Invalid::Empty),
                next: self.first_unsettled(),
                ..Step::default()
            };
        }
        Step {
            changes: vec![Change::Complete {
                response: ChatInputResponseKind::Accept,
                answers: (!answers.is_empty()).then_some(answers),
            }],
            ..Step::default()
        }
    }

    /// An answer in words, for the log.
    pub fn answer_text_for(&self, field: &Field) -> Option<String> {
        let answer = self.answers.get(&field.id)?;
        let value = match answer {
            ChatInputAnswer::Skipped(_) => {
                return Some(otto_kit::t_owned!("launcher-input-skipped"))
            }
            ChatInputAnswer::Draft(_) => return None,
            ChatInputAnswer::Submitted(answered) => &answered.value,
        };
        let label = |id: &str| match &field.kind {
            FieldKind::Single { options, .. } | FieldKind::Multi { options, .. } => options
                .iter()
                .find(|option| option.id == id)
                .map(|option| option.label.clone())
                .unwrap_or_else(|| id.to_string()),
            _ => id.to_string(),
        };
        Some(match value {
            ChatInputAnswerValue::Text(text) => text.value.clone(),
            ChatInputAnswerValue::Number(number) => format_number(number.value),
            ChatInputAnswerValue::Boolean(b) => {
                if b.value {
                    otto_kit::t_owned!("launcher-input-yes")
                } else {
                    otto_kit::t_owned!("launcher-input-no")
                }
            }
            ChatInputAnswerValue::Selected(one) => label(&one.value),
            ChatInputAnswerValue::SelectedMany(many) => {
                let typed = many.freeform_values.iter().flatten().cloned();
                many.value
                    .iter()
                    .map(|id| label(id))
                    .chain(typed)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
            ChatInputAnswerValue::Unknown(_) => return None,
        })
    }

    /// What the log says about the request: its message and link, the answers
    /// so far, and — while it is open — question `current` in full. `invalid`
    /// says why the last answer was not taken.
    pub fn lines(&self, current: Option<usize>, invalid: Option<&str>) -> Vec<(String, Style)> {
        let mut lines = Vec::new();
        let open = self.is_open();
        if let Some(message) = &self.message {
            let style = if open { Style::Prompt } else { Style::Answer };
            lines.push((message.clone(), style));
        }
        if let Some(url) = &self.url {
            lines.push((url.clone(), if open { Style::Answer } else { Style::Note }));
        }
        let current = current.filter(|_| open);
        for (index, field) in self.fields.iter().enumerate() {
            if Some(index) == current {
                if self.fields.len() > 1 {
                    lines.push((
                        otto_kit::t_owned!(
                            "launcher-input-progress",
                            current = (index + 1).to_string(),
                            total = self.fields.len().to_string()
                        ),
                        Style::Note,
                    ));
                }
                match &field.title {
                    Some(title) => {
                        lines.push((title.clone(), Style::Prompt));
                        lines.push((field.message.clone(), Style::Answer));
                    }
                    None => lines.push((field.message.clone(), Style::Prompt)),
                }
                if let Some(invalid) = invalid {
                    lines.push((invalid.to_string(), Style::Note));
                }
            } else if let Some(answer) = self.answer_text_for(field) {
                lines.push((
                    otto_kit::t_owned!(
                        "launcher-input-answer",
                        question = field.label(),
                        answer = answer.as_str()
                    ),
                    Style::Note,
                ));
            }
        }
        if current.is_none() && open {
            if let Some(invalid) = invalid {
                lines.push((invalid.to_string(), Style::Note));
            }
        }
        let outcome = match self.outcome {
            Outcome::Responded(ChatInputResponseKind::Decline) => {
                Some(otto_kit::t_owned!("launcher-input-declined"))
            }
            Outcome::Responded(ChatInputResponseKind::Cancel) => {
                Some(otto_kit::t_owned!("launcher-input-dismissed"))
            }
            Outcome::Unanswered => Some(otto_kit::t_owned!("launcher-input-unanswered")),
            _ => None,
        };
        lines.extend(outcome.map(|outcome| (outcome, Style::Note)));
        lines
    }
}

/// Open `url` in the person's browser, in a process group of its own so it
/// outlives the launcher.
pub fn open_link(url: &str) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn part(value: Value) -> InputRequestResponsePart {
        serde_json::from_value(value).expect("an input request part")
    }

    /// The questions Claude asks with AskUserQuestion, and an MCP elicitation's
    /// kinds, as the spec's JSON carries them.
    fn survey(answers: Value) -> InputRequest {
        InputRequest::from_part(
            &part(json!({
                "request": {
                    "id": "req-1",
                    "message": "A few things before I start",
                    "questions": [
                        { "kind": "single-select", "id": "db", "title": "Database",
                          "message": "Which database?", "required": true,
                          "allowFreeformInput": true,
                          "options": [
                            { "id": "pg", "label": "Postgres", "description": "Relational" },
                            { "id": "sqlite", "label": "SQLite", "recommended": true }
                          ] },
                        { "kind": "multi-select", "id": "features", "message": "Which features?",
                          "options": [
                            { "id": "auth", "label": "Auth" },
                            { "id": "billing", "label": "Billing" }
                          ] },
                        { "kind": "integer", "id": "port", "message": "Port?", "min": 1, "max": 65535 },
                        { "kind": "boolean", "id": "tests", "message": "Write tests?", "required": true },
                        { "kind": "text", "id": "name", "message": "Project name", "defaultValue": "demo" },
                        { "kind": "someday-a-new-kind", "id": "later", "message": "?" }
                    ],
                    "answers": answers
                }
            })),
            true,
        )
    }

    fn json_of(answer: &ChatInputAnswer) -> Value {
        serde_json::to_value(answer).unwrap()
    }

    #[test]
    fn a_request_reads_every_kind_it_can_answer() {
        let request = survey(json!({}));
        assert!(request.is_open());
        assert_eq!(
            request.message.as_deref(),
            Some("A few things before I start")
        );
        let ids: Vec<&str> = request.fields.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            ["db", "features", "port", "tests", "name"],
            "unknown kinds are left out"
        );
        assert!(matches!(
            request.fields[0].kind,
            FieldKind::Single { freeform: true, .. }
        ));
        assert!(matches!(
            request.fields[2].kind,
            FieldKind::Number { integer: true, .. }
        ));
        assert_eq!(request.fields[0].label(), "Database");
        assert_eq!(request.fields[1].label(), "Which features?");
        assert_eq!(request.first_unsettled(), Some(0));

        // Options, then no skip for a required question, then decline.
        assert_eq!(
            request.rows(Some(0)),
            [Row::Option(0), Row::Option(1), Row::Decline]
        );
        assert_eq!(request.default_row(Some(0)), 1, "the suggested option");
        assert_eq!(
            request.rows(Some(1)),
            [
                Row::Option(0),
                Row::Option(1),
                Row::Continue,
                Row::Skip,
                Row::Decline
            ]
        );
        assert_eq!(request.rows(Some(3)), [Row::Yes, Row::No, Row::Decline]);
        assert_eq!(
            request.rows(Some(4)),
            [Row::Continue, Row::Skip, Row::Decline]
        );
        assert_eq!(request.prefill(Some(4)).as_deref(), Some("demo"));
    }

    #[test]
    fn picking_an_option_submits_it_and_moves_on() {
        let request = survey(json!({}));
        let step = request.choose(Row::Option(1), Some(0), "");
        assert_eq!(step.next, Some(1));
        let [Change::Answer {
            question_id,
            answer,
        }] = step.changes.as_slice()
        else {
            panic!("one answer: {:?}", step.changes);
        };
        assert_eq!(question_id, "db");
        assert_eq!(
            json_of(answer),
            json!({ "state": "submitted", "value": { "kind": "selected", "value": "sqlite" } })
        );
    }

    #[test]
    fn a_typed_answer_stands_in_for_an_option() {
        let request = survey(json!({}));
        let step = request.answer_text(Some(0), " MariaDB ");
        assert!(step.took_text);
        let Change::Answer { answer, .. } = &step.changes[0] else {
            panic!()
        };
        assert_eq!(
            json_of(answer),
            json!({ "state": "submitted", "value": {
                "kind": "selected", "value": "MariaDB", "freeformValues": ["MariaDB"] } })
        );
    }

    #[test]
    fn multi_select_toggles_drafts_then_submits_on_continue() {
        let request = survey(json!({}));
        let step = request.choose(Row::Option(1), Some(1), "");
        assert_eq!(step.next, Some(1), "stays on the question");
        let Change::Answer { answer, .. } = &step.changes[0] else {
            panic!()
        };
        assert_eq!(
            json_of(answer),
            json!({ "state": "draft", "value": { "kind": "selected-many", "value": ["billing"] } })
        );

        // The draft came back from the service: picking again unticks it.
        let request = survey(json!({
            "features": { "state": "draft", "value": { "kind": "selected-many", "value": ["billing"] } }
        }));
        assert_eq!(request.row_text(Row::Option(1), Some(1)).0, "☑ Billing");
        assert_eq!(request.row_text(Row::Option(0), Some(1)).0, "☐ Auth");
        let step = request.choose(Row::Option(1), Some(1), "");
        let Change::Answer { answer, .. } = &step.changes[0] else {
            panic!()
        };
        assert_eq!(json_of(answer)["value"]["value"], json!([]));

        let step = request.choose(Row::Continue, Some(1), "Search");
        assert!(step.took_text);
        assert_eq!(step.next, Some(2));
        let Change::Answer { answer, .. } = &step.changes[0] else {
            panic!()
        };
        assert_eq!(
            json_of(answer),
            json!({ "state": "submitted", "value": {
                "kind": "selected-many", "value": ["billing"], "freeformValues": ["Search"] } })
        );
    }

    #[test]
    fn numbers_are_checked_before_they_are_sent() {
        let request = survey(json!({}));
        assert_eq!(
            request.answer_text(Some(2), "abc").invalid,
            Some(Invalid::NotNumber)
        );
        assert_eq!(
            request.answer_text(Some(2), "8,5").invalid,
            Some(Invalid::NotInteger)
        );
        assert_eq!(
            request.answer_text(Some(2), "70000").invalid,
            Some(Invalid::Above("65535".into()))
        );
        // Nothing typed on an optional question skips it.
        let step = request.answer_text(Some(2), "");
        let Change::Answer { answer, .. } = &step.changes[0] else {
            panic!()
        };
        assert_eq!(json_of(answer), json!({ "state": "skipped" }));

        let step = request.answer_text(Some(2), "8080");
        let Change::Answer { answer, .. } = &step.changes[0] else {
            panic!()
        };
        assert_eq!(
            json_of(answer),
            json!({ "state": "submitted", "value": { "kind": "number", "value": 8080.0 } })
        );
    }

    #[test]
    fn the_last_answer_sends_the_request_with_every_answer() {
        let request = survey(json!({
            "db": { "state": "submitted", "value": { "kind": "selected", "value": "pg" } },
            "features": { "state": "skipped" },
            "port": { "state": "submitted", "value": { "kind": "number", "value": 22 } },
            "tests": { "state": "draft", "value": { "kind": "boolean", "value": false } }
        }));
        assert_eq!(
            request.first_unsettled(),
            Some(3),
            "a draft is not an answer"
        );
        assert_eq!(request.default_row(Some(3)), 1, "the draft's No");

        let step = request.choose(Row::Yes, Some(3), "");
        assert_eq!(step.next, Some(4));

        let request = survey(json!({
            "db": { "state": "submitted", "value": { "kind": "selected", "value": "pg" } },
            "features": { "state": "skipped" },
            "port": { "state": "submitted", "value": { "kind": "number", "value": 22 } },
            "tests": { "state": "submitted", "value": { "kind": "boolean", "value": true } },
            "name": { "state": "draft", "value": { "kind": "text", "value": "ot" } }
        }));
        assert_eq!(
            request.prefill(Some(4)).as_deref(),
            Some("ot"),
            "the draft over the default"
        );
        let step = request.answer_text(Some(4), "otto");
        let [Change::Answer { .. }, Change::Complete { response, answers }] =
            step.changes.as_slice()
        else {
            panic!("an answer then the completion: {:?}", step.changes);
        };
        assert_eq!(*response, ChatInputResponseKind::Accept);
        let answers = serde_json::to_value(answers).unwrap();
        assert_eq!(
            answers["name"]["value"],
            json!({ "kind": "text", "value": "otto" })
        );
        assert_eq!(answers["db"]["state"], "submitted");
        assert_eq!(answers["features"], json!({ "state": "skipped" }));
        assert_eq!(answers.as_object().unwrap().len(), 5);
    }

    #[test]
    fn a_required_question_left_open_keeps_the_request_open() {
        let request = survey(json!({}));
        // Skipping to the end and answering it cannot send: db is required.
        let step = request.answer_text(Some(4), "otto");
        assert_eq!(step.invalid, Some(Invalid::Empty));
        assert_eq!(step.next, Some(0));
        assert_eq!(step.changes.len(), 1, "the answer is still shared");
    }

    #[test]
    fn declining_sends_no_answers() {
        let request = survey(json!({}));
        let step = request.choose(Row::Decline, Some(2), "");
        assert_eq!(
            step.changes,
            [Change::Complete {
                response: ChatInputResponseKind::Decline,
                answers: None
            }]
        );
    }

    #[test]
    fn a_link_request_opens_the_link_and_is_done() {
        let request = InputRequest::from_part(
            &part(json!({ "request": {
                "id": "auth", "message": "Sign in to continue", "url": "https://example.com/login" } })),
            true,
        );
        assert_eq!(request.first_unsettled(), None);
        assert_eq!(request.rows(None), [Row::OpenLink, Row::Send, Row::Decline]);
        assert_eq!(request.default_row(None), 1);
        assert_eq!(
            request.row_text(Row::Send, None).0,
            otto_kit::t_owned!("launcher-input-done")
        );
        assert_eq!(
            request.choose(Row::OpenLink, None, "").open.as_deref(),
            Some("https://example.com/login")
        );
        assert_eq!(
            request.choose(Row::Send, None, "").changes,
            [Change::Complete {
                response: ChatInputResponseKind::Accept,
                answers: None
            }]
        );

        // Any other scheme is shown, never opened.
        let odd = InputRequest::from_part(
            &part(json!({ "request": { "id": "x", "url": "file:///etc/passwd" } })),
            true,
        );
        assert_eq!(odd.rows(None), [Row::Send, Row::Decline]);
    }

    #[test]
    fn a_settled_request_collapses_to_its_answers() {
        let answered = InputRequest::from_part(
            &part(json!({
                "request": {
                    "id": "req-1",
                    "questions": [
                        { "kind": "single-select", "id": "db", "message": "Which database?",
                          "options": [{ "id": "pg", "label": "Postgres" }] },
                        { "kind": "boolean", "id": "tests", "message": "Write tests?" }
                    ],
                    "answers": {
                        "db": { "state": "submitted", "value": { "kind": "selected", "value": "pg" } },
                        "tests": { "state": "skipped" }
                    }
                },
                "response": "accept"
            })),
            true,
        );
        assert!(!answered.is_open());
        let lines: Vec<(String, Style)> = answered.lines(Some(0), None);
        assert_eq!(
            lines,
            [
                (
                    otto_kit::t_owned!(
                        "launcher-input-answer",
                        question = "Which database?",
                        answer = "Postgres"
                    ),
                    Style::Note
                ),
                (
                    otto_kit::t_owned!(
                        "launcher-input-answer",
                        question = "Write tests?",
                        answer = otto_kit::t_owned!("launcher-input-skipped")
                    ),
                    Style::Note
                ),
            ]
        );

        let unanswered = InputRequest::from_part(
            &part(json!({ "request": { "id": "r", "message": "Pick one" } })),
            false,
        );
        assert_eq!(unanswered.outcome, Outcome::Unanswered);
        assert_eq!(
            unanswered.lines(None, None).last().map(|l| l.0.clone()),
            Some(otto_kit::t_owned!("launcher-input-unanswered"))
        );
    }

    #[test]
    fn the_open_question_is_shown_in_full_with_what_is_wrong() {
        let request = survey(json!({
            "db": { "state": "submitted", "value": { "kind": "selected", "value": "sqlite" } }
        }));
        let lines = request.lines(Some(2), Some("That needs to be a whole number"));
        let texts: Vec<&str> = lines.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(texts[0], "A few things before I start");
        assert!(texts[1].contains("SQLite"));
        assert_eq!(lines[3], ("Port?".to_string(), Style::Prompt));
        assert_eq!(texts[4], "That needs to be a whole number");
    }
}
