pub mod template;

use std::{collections::HashMap, str, vec::Vec};

use chrono::NaiveDate;
use regex::{Regex, RegexSet, RegexSetBuilder};
use serde::{Deserialize, Serialize};

use crate::template::interpolate;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecimalSeparator {
    Period,
    Comma,
}

impl DecimalSeparator {
    pub fn simplify(&self, s: &str) -> String {
        match self {
            Self::Period => s.replace(",", ""),
            Self::Comma => s.replace(".", "").replace(",", "."),
        }
    }
}

/// See: https://docs.youneedabudget.com/article/921-formatting-csv-file
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type", content = "args")]
pub enum Field {
    Ignore,
    Date(String),
    Payee,
    Category,
    Memo,
    Inflow(DecimalSeparator),
    Outflow(DecimalSeparator),
    Extra(String),
}

#[derive(Debug)]
pub struct Record {
    pub date: String,
    pub payee: String,
    pub category: String,
    pub memo: String,
    pub amount: String,
    pub extra: HashMap<String, String>,
    pub transformed: bool,
}

impl Record {
    pub fn new() -> Self {
        Self {
            date: String::new(),
            payee: String::new(),
            category: String::new(),
            memo: String::new(),
            amount: String::new(),
            extra: HashMap::new(),
            transformed: false,
        }
    }

    pub fn from<'a, 'b>(
        input: &'a csv::StringRecord,
        mapping: impl IntoIterator<Item = &'b Field>,
    ) -> Self {
        let mut r = Self::new();
        for (i, col) in mapping.into_iter().enumerate() {
            let v = input
                .get(i)
                .expect("input record has less columns than expected")
                .to_owned();
            match col {
                Field::Ignore => continue,
                Field::Date(format) => {
                    if format.is_empty() {
                        r.date = v;
                    } else {
                        let date = NaiveDate::parse_from_str(&v, format)
                            .expect("date or date format is malformed");
                        r.date = date.format("%Y-%m-%d").to_string();
                    }
                }
                Field::Payee => r.payee = v,
                Field::Category => r.category = v,
                Field::Memo => r.memo = v,
                Field::Inflow(sep) => r.amount = sep.simplify(&v),
                Field::Outflow(sep) => r.amount = format!("-{}", sep.simplify(&v)),
                Field::Extra(key) => {
                    r.extra.insert(key.to_owned(), v);
                }
            }
        }
        r
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        match key {
            "date" => Some(&self.date),
            "payee" => Some(&self.payee),
            "category" => Some(&self.category),
            "memo" => Some(&self.memo),
            "amount" => Some(&self.amount),
            key => self.extra.get(key).map(|x| x.as_str()),
        }
    }

    /// Moves value into location in the record indicated by key,
    /// returning the previous dest value, or None if the key did not exist before
    /// (this only applies to non-standard fields).
    ///
    /// Neither value is dropped.
    pub fn replace(&mut self, key: &str, value: String) -> Option<String> {
        match key {
            "date" => Some(std::mem::replace(&mut self.date, value)),
            "payee" => Some(std::mem::replace(&mut self.payee, value)),
            "category" => Some(std::mem::replace(&mut self.category, value)),
            "memo" => Some(std::mem::replace(&mut self.memo, value)),
            "amount" => Some(std::mem::replace(&mut self.amount, value)),
            key => self.extra.insert(key.to_string(), value),
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        vec!["date", "payee", "category", "memo", "amount"]
            .into_iter()
            .chain(self.extra.keys().map(|x| x.as_str()))
    }
}

pub trait YnabRecord {
    fn header() -> csv::StringRecord;
    fn to_record(&self) -> csv::StringRecord;
}

impl YnabRecord for Record {
    fn header() -> csv::StringRecord {
        csv::StringRecord::from(vec!["Date", "Payee", "Category", "Memo", "Amount"])
    }

    fn to_record(&self) -> csv::StringRecord {
        csv::StringRecord::from(vec![
            &self.date,
            &self.payee,
            &self.category,
            &self.memo,
            &self.amount,
        ])
    }
}

pub trait Transformer {
    fn is_match(&self, record: &Record) -> bool;
    fn transform(&self, record: &mut Record) -> bool;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MatcherBuilder {
    pub label: Option<String>,
    #[serde(rename = "match")]
    pub search: HashMap<String, String>,
    pub replace: HashMap<String, String>,
}

impl MatcherBuilder {
    pub fn build(self) -> Matcher {
        Matcher {
            search: HashMap::from_iter(
                self.search
                    .into_iter()
                    .map(|(k, v)| (k, Regex::new(&v).unwrap())),
            ),
            replace: self.replace,
        }
    }
}

impl From<MatcherBuilder> for Matcher {
    fn from(builder: MatcherBuilder) -> Matcher {
        builder.build()
    }
}

#[derive(Debug)]
pub struct Matcher {
    search: HashMap<String, Regex>,
    replace: HashMap<String, String>,
}

impl Default for Matcher {
    fn default() -> Self {
        Self {
            search: HashMap::new(),
            replace: HashMap::new(),
        }
    }
}

impl Transformer for Matcher {
    fn is_match(&self, record: &Record) -> bool {
        self.search.iter().all(|(k, v)| match record.get(k) {
            Some(field) => v.is_match(field),
            None => false,
        })
    }

    fn transform(&self, record: &mut Record) -> bool {
        // Check if all search items match and collect their captures into one hash map.
        let mut captures: HashMap<String, String> = HashMap::new();
        for (k, v) in &self.search {
            match record.get(&k) {
                Some(field) => {
                    if let Some(rc) = v.captures(&field) {
                        // Put named captures into the captures hash map.
                        // The filter_map filters out unnamed captures.
                        for n in v.capture_names().filter_map(|x| x) {
                            if let Some(g) = rc.name(n) {
                                captures.insert(n.into(), g.as_str().to_owned());
                            }
                        }
                    } else {
                        return false;
                    }
                }
                None => {
                    return false;
                }
            }
        }

        for (k, v) in &self.replace {
            record.replace(
                k,
                interpolate(v, |key: &str| {
                    captures
                        .get(key)
                        .map(|x| x.to_string())
                        .or_else(|| record.get(key).map(|x| x.to_string()))
                        .unwrap_or_default()
                }),
            );
        }

        record.transformed = true;
        true
    }
}

impl Transformer for Vec<Matcher> {
    fn is_match(&self, r: &Record) -> bool {
        self.iter().any(|x| x.is_match(r))
    }

    fn transform(&self, r: &mut Record) -> bool {
        self.iter().fold(false, |a, x| x.transform(r) | a)
    }
}

pub struct Payees {
    aliases: HashMap<String, RegexSet>,

    /// Print warnings if more than one alias matches.
    strict: bool,
}

impl Payees {
    pub fn new(map: &HashMap<String, Vec<String>>, case_insensitive: bool) -> Self {
        Self {
            aliases: map
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_owned(),
                        RegexSetBuilder::new(v.iter().map(|x| maybe_escape(x.to_owned())))
                            .case_insensitive(case_insensitive)
                            .build()
                            .expect("invalid regular expression"),
                    )
                })
                .collect(),
            strict: false,
        }
    }
}

impl Default for Payees {
    fn default() -> Self {
        Self {
            aliases: HashMap::new(),
            strict: true,
        }
    }
}

impl From<HashMap<String, Vec<String>>> for Payees {
    fn from(map: HashMap<String, Vec<String>>) -> Self {
        Payees::new(&map, false)
    }
}

impl Transformer for Payees {
    fn is_match(&self, r: &Record) -> bool {
        self.aliases.iter().any(|(_, set)| set.is_match(&r.payee))
    }

    fn transform(&self, r: &mut Record) -> bool {
        let mut matches = 0;
        let original = r.payee.clone();
        for (k, set) in &self.aliases {
            if set.is_match(&original) {
                matches += 1;
                if self.strict {
                    if matches == 1 {
                        r.payee = k.clone();
                    } else if matches == 2 {
                        eprintln!("warning: multiple aliases match payee: {}", original);
                        eprintln!("       | - {}", r.payee);
                        eprintln!("       | - {}", k);
                    } else {
                        eprintln!("       | - {}", k)
                    }
                } else {
                    r.payee = k.clone();
                    break;
                }
            }
        }
        matches != 0
    }
}

fn maybe_escape(s: String) -> String {
    if s.starts_with('^') && s.ends_with('$') {
        // This string should be treated as a regular expression, return as is.
        s
    } else {
        // This string should be interpreted to be a literal, escape it.
        regex::escape(&s)
    }
}
