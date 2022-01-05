use inflector::Inflector;
use lazy_static::lazy_static;
use regex::Regex;

pub fn interpolate(tmpl: &str, func: impl Fn(&str) -> String) -> String {
    lazy_static! {
        static ref PLACEHOLDER: Regex =
            Regex::new(r"(?mi)\$\{([[:word:]]+)(\|([[:word:]]+))?\}").unwrap();
    }

    if !PLACEHOLDER.is_match(tmpl) {
        return tmpl.to_owned();
    }

    let mut buffer: Vec<String> = Vec::new();
    let mut index: usize = 0;
    for placeholder in PLACEHOLDER.captures_iter(tmpl) {
        let m = placeholder.get(0).unwrap(); // Match 0 always exists.
        buffer.push(tmpl[index..m.start()].into());
        index = m.end();

        let key = placeholder.get(1).unwrap().as_str(); // This group not optional.
        let mut value = func(key);

        if let Some(command) = placeholder.get(3) {
            value = match command.as_str() {
                "title_case" => value.to_title_case(),
                "lowercase" => value.to_lowercase(),
                "uppercase" => value.to_uppercase(),
                "not_empty" => {
                    if value.is_empty() {
                        panic!("value of key {} cannot be empty", key);
                    } else {
                        value
                    }
                }
                invalid => panic!("invalid command: {}", invalid),
            };
        }

        buffer.push(value);
    }

    if index < tmpl.len() {
        buffer.push(tmpl[index..].into());
    }

    buffer.join("")
}

#[cfg(test)]
mod tests {
    use super::interpolate;
    use std::collections::HashMap;

    #[test]
    fn test_interpolate() {
        let words = HashMap::from([("a", "hello"), ("b", "world")]);
        let func = |s: &str| words.get(s).map(|x| x.to_string()).unwrap_or_default();
        let tests = [
            ("${a} ${b}!", "hello world!"),
            ("${a|title_case} ${b}!", "Hello world!"),
            ("${a|not_empty} ${b|uppercase}!", "hello WORLD!"),
        ];

        for test in tests {
            assert_eq!(interpolate(test.0, func), test.1.to_string());
        }
    }
}
