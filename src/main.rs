use std::{collections::HashMap, path::Path, str, vec::Vec};

use clap::{App, Arg};
use encoding::all::ISO_8859_1;
use encoding::{DecoderTrap, Encoding};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_regex;
use serde_yaml::{self};
use thiserror::Error;

use ynap::{Field, Matcher, MatcherBuilder, Payees, Record, Transformer, YnabRecord};

#[derive(Error, Debug)]
pub enum AppError {
    #[error("error parsing CSV file: {}", .0)]
    Csv(#[from] csv::Error),

    #[error("input/output error: {}", .0)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Parser {
    pub name: String,
    #[serde(with = "serde_regex")]
    pub file_pattern: Option<Regex>,
    #[serde(with = "serde_regex")]
    pub ignore_patterns: Vec<Regex>,
    pub ignore_header_rows: usize,
    pub delimiter: String,
    pub columns: Vec<Field>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub pre_transform: Vec<MatcherBuilder>,
    #[serde(default)]
    pub payees: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub post_transform: Vec<MatcherBuilder>,
}

pub struct MrClean {
    transformers: Vec<Box<dyn Transformer>>,
}

impl From<Rules> for MrClean {
    fn from(r: Rules) -> Self {
        let mut tx: Vec<Box<dyn Transformer>> =
            Vec::with_capacity(r.pre_transform.len() + r.post_transform.len() + 1);
        for b in r.pre_transform {
            tx.push(Box::new(Matcher::from(b)));
        }
        tx.push(Box::new(Payees::new(&r.payees, true)));
        for b in r.post_transform {
            tx.push(Box::new(Matcher::from(b)));
        }
        Self { transformers: tx }
    }
}

impl Transformer for MrClean {
    fn is_match(&self, r: &Record) -> bool {
        self.transformers.iter().any(|x| x.is_match(r))
    }

    fn transform(&self, r: &mut Record) -> bool {
        self.transformers
            .iter()
            .fold(false, |a, x| x.transform(r) | a)
    }
}

impl Parser {
    pub fn read_from_path(&self, path: impl AsRef<Path>) -> Result<Vec<Record>, AppError> {
        let bytes = std::fs::read(path)?;
        let input = match std::str::from_utf8(&bytes) {
            Ok(v) => v.into(),
            Err(_) => ISO_8859_1.decode(&bytes, DecoderTrap::Replace).unwrap(),
        };

        self.read_from_string(input)
    }

    pub fn read_from_string(&self, s: String) -> Result<Vec<Record>, AppError> {
        // Preprocess the entire input.
        let input: String = s
            .lines()
            .skip(self.ignore_header_rows)
            .filter(|x| !self.ignore_patterns.iter().any(|p| p.is_match(x)))
            .map(|s| format!("{}\n", s))
            .collect();

        // Convert the CSV records into ynap::Records.
        let records = csv::ReaderBuilder::new()
            .delimiter(self.delimiter.as_bytes()[0])
            .has_headers(false)
            .from_reader(input.as_bytes())
            .records()
            .map(|x| Record::from(&x.expect("invalid line in file"), self.columns.iter()))
            .collect();

        Ok(records)
    }
}

fn main() -> Result<(), AppError> {
    let matches = App::new("ynap")
        .version("0.1")
        .arg(
            Arg::with_name("INPUT")
                .help("Sets the input CSV data file to use")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::with_name("output")
                .short("o")
                .long("output")
                .value_name("PATH")
                .takes_value(true)
                .help("Write output CSV to a file"),
        )
        .arg(
            Arg::with_name("bank")
                .short("b")
                .long("bank")
                .value_name("PATH")
                .takes_value(true)
                .required(true)
                .help("Specify bank file format"),
        )
        .arg(
            Arg::with_name("rules")
                .short("r")
                .long("rules")
                .value_name("PATH")
                .takes_value(true)
                .help("Read transformation rules for processing records"),
        )
        .arg(
            Arg::with_name("conf")
                .short("c")
                .long("conf")
                .value_name("PATH")
                .takes_value(true)
                .help("Configuration file"),
        )
        .get_matches();

    let bank_file = matches.value_of("bank").unwrap();
    let bank_file = std::fs::File::open(bank_file).expect("could not open file");
    let bank: Parser = serde_yaml::from_reader(bank_file).expect("could not parse YAML bank file");
    let mut results = bank.read_from_path(matches.value_of("INPUT").unwrap())?;

    if let Some(rules_path) = matches.value_of("rules") {
        let f = std::fs::File::open(rules_path).expect("could not open file");
        let rules: Rules = serde_yaml::from_reader(f).expect("could not parse YAML");
        let mr_clean = MrClean::from(rules);
        results = results
            .into_iter()
            .map(|mut r| {
                mr_clean.transform(&mut r);
                r
            })
            .collect();
    }

    let mut wrt = csv::Writer::from_writer(std::io::stdout());
    wrt.write_record(&Record::header())?;
    for rec in results {
        let rec: csv::StringRecord = (&rec).to_record();
        wrt.write_record(&rec)?;
    }

    Ok(())
}
