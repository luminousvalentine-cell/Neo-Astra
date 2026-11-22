#[derive(Clone, Debug, PartialEq)]
pub enum OrderedJson {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<OrderedJson>),
    Object(Vec<(String, OrderedJson)>),
}

impl OrderedJson {
    pub fn parse(input: &[u8]) -> Result<Self, String> {
        let mut parser = Parser { input, index: 0 };
        let value = parser.value()?;
        parser.whitespace();
        if parser.index != input.len() {
            return Err("Unexpected trailing JSON content.".to_owned());
        }
        Ok(value)
    }

    pub fn get(&self, key: &str) -> Option<&OrderedJson> {
        match self {
            OrderedJson::Object(entries) => entries
                .iter()
                .find(|(entry, _)| entry == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[OrderedJson]> {
        match self {
            OrderedJson::Array(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn is_object(&self) -> bool {
        matches!(self, OrderedJson::Object(_))
    }

    pub fn to_pretty(&self) -> String {
        let mut output = String::new();
        write_pretty(self, 0, &mut output);
        output
    }
}

fn write_pretty(value: &OrderedJson, indent: usize, output: &mut String) {
    match value {
        OrderedJson::Null => output.push_str("null"),
        OrderedJson::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        OrderedJson::Number(value) => output.push_str(value),
        OrderedJson::String(value) => {
            output.push_str(&serde_json::to_string(value).unwrap_or_default());
        }
        OrderedJson::Array(entries) => {
            if entries.is_empty() {
                output.push_str("[]");
                return;
            }
            output.push_str("[\n");
            for (index, entry) in entries.iter().enumerate() {
                output.push_str(&" ".repeat(indent + 2));
                write_pretty(entry, indent + 2, output);
                if index + 1 < entries.len() {
                    output.push(',');
                }
                output.push('\n');
            }
            output.push_str(&" ".repeat(indent));
            output.push(']');
        }
        OrderedJson::Object(entries) => {
            if entries.is_empty() {
                output.push_str("{}");
                return;
            }
            output.push_str("{\n");
            for (index, (key, entry)) in entries.iter().enumerate() {
                output.push_str(&" ".repeat(indent + 2));
                output.push_str(&serde_json::to_string(key).unwrap_or_default());
                output.push_str(": ");
                write_pretty(entry, indent + 2, output);
                if index + 1 < entries.len() {
                    output.push(',');
                }
                output.push('\n');
            }
            output.push_str(&" ".repeat(indent));
            output.push('}');
        }
    }
}

pub fn js_string(value: Option<&OrderedJson>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(OrderedJson::Null) => "null".to_owned(),
        Some(OrderedJson::Bool(value)) => value.to_string(),
        Some(OrderedJson::Number(value)) => value.clone(),
        Some(OrderedJson::String(value)) => value.clone(),
        Some(OrderedJson::Array(entries)) => entries
            .iter()
            .map(|entry| match entry {
                OrderedJson::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(OrderedJson::Object(_)) => "[object Object]".to_owned(),
    }
}

struct Parser<'a> {
    input: &'a [u8],
    index: usize,
}

impl Parser<'_> {
    fn value(&mut self) -> Result<OrderedJson, String> {
        self.whitespace();
        let Some(byte) = self.input.get(self.index).copied() else {
            return Err("Unexpected end of JSON.".to_owned());
        };
        match byte {
            b'n' => {
                self.literal(b"null")?;
                Ok(OrderedJson::Null)
            }
            b't' => {
                self.literal(b"true")?;
                Ok(OrderedJson::Bool(true))
            }
            b'f' => {
                self.literal(b"false")?;
                Ok(OrderedJson::Bool(false))
            }
            b'"' => Ok(OrderedJson::String(self.string()?)),
            b'[' => self.array(),
            b'{' => self.object(),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err("Invalid JSON value.".to_owned()),
        }
    }

    fn literal(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.input.get(self.index..self.index + literal.len()) == Some(literal) {
            self.index += literal.len();
            Ok(())
        } else {
            Err("Invalid JSON literal.".to_owned())
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.index += 1;
        let mut output = String::new();
        while let Some(byte) = self.input.get(self.index).copied() {
            self.index += 1;
            match byte {
                b'"' => return Ok(output),
                b'\\' => {
                    let Some(escape) = self.input.get(self.index).copied() else {
                        return Err("Unterminated JSON escape.".to_owned());
                    };
                    self.index += 1;
                    match escape {
                        b'"' => output.push('"'),
                        b'\\' => output.push('\\'),
                        b'/' => output.push('/'),
                        b'b' => output.push('\u{0008}'),
                        b'f' => output.push('\u{000c}'),
                        b'n' => output.push('\n'),
                        b'r' => output.push('\r'),
                        b't' => output.push('\t'),
                        b'u' => output.push(self.unicode_escape()?),
                        _ => return Err("Invalid JSON escape.".to_owned()),
                    }
                }
                byte if byte < 0x20 => {
                    return Err("Unescaped control character in JSON.".to_owned());
                }
                byte => {
                    if byte < 0x80 {
                        output.push(byte as char);
                    } else {
                        let start = self.index - 1;
                        let width = utf8_width(byte);
                        let end = start + width;
                        let Some(slice) = self.input.get(start..end) else {
                            return Err("Invalid UTF-8 in JSON.".to_owned());
                        };
                        let Ok(text) = std::str::from_utf8(slice) else {
                            return Err("Invalid UTF-8 in JSON.".to_owned());
                        };
                        output.push_str(text);
                        self.index = end;
                    }
                }
            }
        }
        Err("Unterminated JSON string.".to_owned())
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let high = self.hex4()?;
        if (0xD800..=0xDBFF).contains(&high) {
            if self.input.get(self.index..self.index + 2) != Some(b"\\u") {
                return Err("Invalid JSON surrogate pair.".to_owned());
            }
            self.index += 2;
            let low = self.hex4()?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err("Invalid JSON surrogate pair.".to_owned());
            }
            let code = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
            return char::from_u32(code).ok_or_else(|| "Invalid JSON code point.".to_owned());
        }
        char::from_u32(high).ok_or_else(|| "Invalid JSON code point.".to_owned())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let Some(slice) = self.input.get(self.index..self.index + 4) else {
            return Err("Invalid JSON unicode escape.".to_owned());
        };
        self.index += 4;
        let text =
            std::str::from_utf8(slice).map_err(|_| "Invalid JSON unicode escape.".to_owned())?;
        u32::from_str_radix(text, 16).map_err(|_| "Invalid JSON unicode escape.".to_owned())
    }

    fn array(&mut self) -> Result<OrderedJson, String> {
        self.index += 1;
        let mut entries = Vec::new();
        self.whitespace();
        if self.input.get(self.index) == Some(&b']') {
            self.index += 1;
            return Ok(OrderedJson::Array(entries));
        }
        loop {
            entries.push(self.value()?);
            self.whitespace();
            match self.input.get(self.index).copied() {
                Some(b',') => {
                    self.index += 1;
                }
                Some(b']') => {
                    self.index += 1;
                    return Ok(OrderedJson::Array(entries));
                }
                _ => return Err("Invalid JSON array.".to_owned()),
            }
        }
    }

    fn object(&mut self) -> Result<OrderedJson, String> {
        self.index += 1;
        let mut entries = Vec::new();
        self.whitespace();
        if self.input.get(self.index) == Some(&b'}') {
            self.index += 1;
            return Ok(OrderedJson::Object(entries));
        }
        loop {
            self.whitespace();
            if self.input.get(self.index) != Some(&b'"') {
                return Err("Invalid JSON object key.".to_owned());
            }
            let key = self.string()?;
            self.whitespace();
            if self.input.get(self.index) != Some(&b':') {
                return Err("Invalid JSON object.".to_owned());
            }
            self.index += 1;
            entries.push((key, self.value()?));
            self.whitespace();
            match self.input.get(self.index).copied() {
                Some(b',') => {
                    self.index += 1;
                }
                Some(b'}') => {
                    self.index += 1;
                    return Ok(OrderedJson::Object(entries));
                }
                _ => return Err("Invalid JSON object.".to_owned()),
            }
        }
    }

    fn number(&mut self) -> Result<OrderedJson, String> {
        let start = self.index;
        if self.input.get(self.index) == Some(&b'-') {
            self.index += 1;
        }
        let Some(first) = self.input.get(self.index).copied() else {
            return Err("Invalid JSON number.".to_owned());
        };
        if first == b'0' {
            self.index += 1;
        } else if first.is_ascii_digit() {
            while self
                .input
                .get(self.index)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                self.index += 1;
            }
        } else {
            return Err("Invalid JSON number.".to_owned());
        }
        if self.input.get(self.index) == Some(&b'.') {
            self.index += 1;
            while self
                .input
                .get(self.index)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                self.index += 1;
            }
        }
        if matches!(self.input.get(self.index), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.input.get(self.index), Some(b'+' | b'-')) {
                self.index += 1;
            }
            while self
                .input
                .get(self.index)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                self.index += 1;
            }
        }
        let text = std::str::from_utf8(&self.input[start..self.index])
            .map_err(|_| "Invalid JSON number.".to_owned())?;
        Ok(OrderedJson::Number(text.to_owned()))
    }

    fn whitespace(&mut self) {
        while self
            .input
            .get(self.index)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.index += 1;
        }
    }
}

fn utf8_width(byte: u8) -> usize {
    if byte < 0x80 {
        1
    } else if byte < 0xE0 {
        2
    } else if byte < 0xF0 {
        3
    } else {
        4
    }
}
