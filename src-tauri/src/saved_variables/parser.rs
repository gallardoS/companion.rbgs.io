//! A bounded reader for the diagnostic addon's data-only Lua serialization.
//! This module has no Lua runtime and never evaluates identifiers or expressions.
use serde::Serialize;
use serde_json::Number;
use std::collections::BTreeMap;

pub const MAX_BYTES: usize = 2_000_000;
const MAX_DEPTH: usize = 30;
const MAX_ITEMS: usize = 200_000;
const MAX_TOKENS: usize = MAX_ITEMS * 8;
const MAX_STRING_BYTES: usize = 16_384;
const MAX_RECORDS: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Integer(i64),
    Text(String),
}

#[derive(Debug)]
enum Value {
    Nil,
    Bool,
    Number(Number),
    Text(String),
    Table(BTreeMap<Key, Value>),
}

impl Value {
    fn table(&self) -> Option<&BTreeMap<Key, Value>> {
        if let Self::Table(value) = self {
            Some(value)
        } else {
            None
        }
    }
    fn get(&self, name: &str) -> Option<&Self> {
        self.table()?.get(&Key::Text(name.into()))
    }
    fn position(&self, index: i64) -> Option<&Self> {
        self.table()?.get(&Key::Integer(index))
    }
    fn text(&self) -> Option<&str> {
        if let Self::Text(value) = self {
            Some(value)
        } else {
            None
        }
    }
    fn integer(&self) -> Option<u64> {
        if let Self::Number(value) = self {
            value.as_u64()
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    TooLarge,
    TooDeep,
    TooManyItems,
    TooManyTokens,
    StringTooLong,
    InvalidEncoding,
    InvalidSyntax,
    DuplicateKey,
    UnsupportedSchema,
    InvalidRecords,
}

type Result<T> = std::result::Result<T, ParseError>;

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
    items: usize,
    tokens: usize,
}

impl<'a> Parser<'a> {
    fn skip(&mut self) -> Result<()> {
        loop {
            while self
                .bytes
                .get(self.offset)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.offset += 1;
            }
            if !self.bytes[self.offset..].starts_with(b"--") {
                return Ok(());
            }
            self.offset += 2;
            if self.bytes[self.offset..].starts_with(b"[[") {
                self.offset += 2;
                while !self.bytes[self.offset..].starts_with(b"]]") {
                    if self.offset == self.bytes.len() {
                        return Err(ParseError::InvalidSyntax);
                    }
                    self.offset += 1;
                }
                self.offset += 2;
            } else {
                while self.bytes.get(self.offset).is_some_and(|b| *b != b'\n') {
                    self.offset += 1;
                }
            }
        }
    }
    fn peek(&mut self) -> Result<Option<u8>> {
        self.skip()?;
        Ok(self.bytes.get(self.offset).copied())
    }
    fn token(&mut self) -> Result<()> {
        self.tokens += 1;
        if self.tokens > MAX_TOKENS {
            return Err(ParseError::TooManyTokens);
        }
        Ok(())
    }
    fn expect(&mut self, byte: u8) -> Result<()> {
        if self.peek()? != Some(byte) {
            return Err(ParseError::InvalidSyntax);
        }
        self.offset += 1;
        self.token()
    }
    fn identifier(&mut self) -> Result<String> {
        self.skip()?;
        let start = self.offset;
        if !self
            .bytes
            .get(start)
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            return Err(ParseError::InvalidSyntax);
        }
        self.offset += 1;
        while self
            .bytes
            .get(self.offset)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            self.offset += 1;
        }
        if self.offset - start > MAX_STRING_BYTES {
            return Err(ParseError::StringTooLong);
        }
        self.token()?;
        Ok(std::str::from_utf8(&self.bytes[start..self.offset])
            .map_err(|_| ParseError::InvalidEncoding)?
            .into())
    }
    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut output = Vec::new();
        loop {
            let byte = *self
                .bytes
                .get(self.offset)
                .ok_or(ParseError::InvalidSyntax)?;
            self.offset += 1;
            match byte {
                b'"' => break,
                b'\n' | b'\r' => return Err(ParseError::InvalidSyntax),
                b'\\' => {
                    let escape = *self
                        .bytes
                        .get(self.offset)
                        .ok_or(ParseError::InvalidSyntax)?;
                    self.offset += 1;
                    match escape {
                        b'"' | b'\\' | b'\'' => output.push(escape),
                        b'a' => output.push(7),
                        b'b' => output.push(8),
                        b'f' => output.push(12),
                        b'n' => output.push(b'\n'),
                        b'r' => output.push(b'\r'),
                        b't' => output.push(b'\t'),
                        b'v' => output.push(11),
                        b'0'..=b'9' => {
                            let mut number = (escape - b'0') as u16;
                            for _ in 0..2 {
                                let Some(digit) =
                                    self.bytes.get(self.offset).filter(|b| b.is_ascii_digit())
                                else {
                                    break;
                                };
                                number = number * 10 + (digit - b'0') as u16;
                                self.offset += 1;
                            }
                            output
                                .push(u8::try_from(number).map_err(|_| ParseError::InvalidSyntax)?);
                        }
                        _ => return Err(ParseError::InvalidSyntax),
                    }
                }
                _ => output.push(byte),
            }
            if output.len() > MAX_STRING_BYTES {
                return Err(ParseError::StringTooLong);
            }
        }
        String::from_utf8(output).map_err(|_| ParseError::InvalidEncoding)
    }
    fn number(&mut self) -> Result<Number> {
        self.skip()?;
        let start = self.offset;
        if self.bytes.get(self.offset) == Some(&b'-') {
            self.offset += 1;
        }
        let digits = self.offset;
        while self.bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
            self.offset += 1;
        }
        if digits == self.offset || (self.bytes[digits] == b'0' && self.offset - digits > 1) {
            return Err(ParseError::InvalidSyntax);
        }
        if self.bytes.get(self.offset) == Some(&b'.') {
            self.offset += 1;
            let decimal = self.offset;
            while self.bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                self.offset += 1;
            }
            if self.offset == decimal {
                return Err(ParseError::InvalidSyntax);
            }
        }
        if self.offset - start > 64 {
            return Err(ParseError::InvalidSyntax);
        }
        self.token()?;
        let literal = std::str::from_utf8(&self.bytes[start..self.offset])
            .map_err(|_| ParseError::InvalidEncoding)?;
        literal.parse().map_err(|_| ParseError::InvalidSyntax)
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(ParseError::TooDeep);
        }
        match self.peek()? {
            Some(b'{') => self.table(depth),
            Some(b'"') => Ok(Value::Text(self.string()?)),
            Some(b'-' | b'0'..=b'9') => Ok(Value::Number(self.number()?)),
            Some(_) => match self.identifier()?.as_str() {
                "nil" => Ok(Value::Nil),
                "true" | "false" => Ok(Value::Bool),
                _ => Err(ParseError::InvalidSyntax),
            },
            None => Err(ParseError::InvalidSyntax),
        }
    }
    fn table(&mut self, depth: usize) -> Result<Value> {
        self.expect(b'{')?;
        let mut table = BTreeMap::new();
        let mut next = 1;
        while self.peek()? != Some(b'}') {
            let key = if self.peek()? == Some(b'[') {
                self.expect(b'[')?;
                let key = match self.value(depth + 1)? {
                    Value::Text(text) => Key::Text(text),
                    Value::Number(number) => {
                        Key::Integer(number.as_i64().ok_or(ParseError::InvalidSyntax)?)
                    }
                    _ => return Err(ParseError::InvalidSyntax),
                };
                self.expect(b']')?;
                self.expect(b'=')?;
                key
            } else if self
                .peek()?
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            {
                let offset = self.offset;
                let name = self.identifier()?;
                if self.peek()? == Some(b'=') {
                    self.expect(b'=')?;
                    Key::Text(name)
                } else {
                    self.offset = offset;
                    let key = Key::Integer(next);
                    next += 1;
                    key
                }
            } else {
                let key = Key::Integer(next);
                next += 1;
                key
            };
            if table.contains_key(&key) {
                return Err(ParseError::DuplicateKey);
            }
            let value = self.value(depth + 1)?;
            self.items += 1;
            if self.items > MAX_ITEMS {
                return Err(ParseError::TooManyItems);
            }
            table.insert(key, value);
            match self.peek()? {
                Some(b',' | b';') => {
                    self.offset += 1;
                    self.token()?;
                }
                Some(b'}') => {}
                _ => return Err(ParseError::InvalidSyntax),
            }
        }
        self.expect(b'}')?;
        Ok(Value::Table(table))
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub schema: u64,
    pub record_count: usize,
    pub client_version: Option<String>,
    pub client_build: Option<String>,
    pub client_interface: Option<String>,
    pub observed_character: Option<String>,
    pub observed_realm: Option<String>,
    pub observed_map: Option<String>,
    pub last_record_at: Option<u64>,
    pub latest_scoreboard_rows: Option<usize>,
    pub raw_winner: Option<String>,
    pub last_capture_phase: Option<String>,
    pub compatibility: Compatibility,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Compatibility {
    ObservedDiagnosticBuild,
    UnsupportedClient,
    UnknownClient,
}

fn display(value: Option<&Value>, maximum: usize) -> Option<String> {
    let value = value?.text()?;
    if value.is_empty()
        || value == "<nil or protected>"
        || value.len() > maximum
        || value.chars().any(char::is_control)
    {
        None
    } else {
        Some(value.into())
    }
}
fn numeric_text(value: Option<&Value>, dots: bool) -> Option<String> {
    let value = display(value, 32)?;
    if value
        .bytes()
        .all(|b| b.is_ascii_digit() || (dots && b == b'.'))
    {
        Some(value)
    } else {
        None
    }
}

fn parse_root(bytes: &[u8], name: &str) -> Result<Value> {
    if bytes.len() > MAX_BYTES {
        return Err(ParseError::TooLarge);
    }
    std::str::from_utf8(bytes).map_err(|_| ParseError::InvalidEncoding)?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let mut parser = Parser {
        bytes,
        offset: 0,
        items: 0,
        tokens: 0,
    };
    if parser.identifier()? != name {
        return Err(ParseError::InvalidSyntax);
    }
    parser.expect(b'=')?;
    let root = parser.value(0)?;
    if parser.peek()?.is_some() {
        return Err(ParseError::InvalidSyntax);
    }
    Ok(root)
}

// Share only the bounded data grammar. Each addon keeps its own root/schema.
pub(super) fn product_json(bytes: &[u8]) -> Result<serde_json::Value> {
    fn convert(value: Value, array: bool) -> Result<serde_json::Value> {
        use serde_json::Value as Json;
        Ok(match value {
            Value::Nil => Json::Null,
            Value::Bool => return Err(ParseError::InvalidRecords),
            Value::Number(value) => Json::Number(value),
            Value::Text(value) => Json::String(value),
            Value::Table(table) if array => {
                let mut rows = Vec::new();
                for (index, (key, value)) in table.into_iter().enumerate() {
                    if key != Key::Integer(index as i64 + 1) {
                        return Err(ParseError::InvalidRecords);
                    }
                    rows.push(convert(value, false)?);
                }
                Json::Array(rows)
            }
            Value::Table(table) => {
                let mut object = serde_json::Map::new();
                for (key, value) in table {
                    let Key::Text(key) = key else {
                        return Err(ParseError::InvalidRecords);
                    };
                    let array = key == "captures" || key == "roster";
                    object.insert(key, convert(value, array)?);
                }
                Json::Object(object)
            }
        })
    }
    convert(parse_root(bytes, "RBGSMatchData")?, false)
}

pub fn parse(bytes: &[u8]) -> Result<Summary> {
    let root = parse_root(bytes, "RBGSCompatibilityData")?;
    if root.get("schema").and_then(Value::integer) != Some(1) {
        return Err(ParseError::UnsupportedSchema);
    }
    let records = root
        .get("records")
        .and_then(Value::table)
        .ok_or(ParseError::InvalidRecords)?;
    if records.len() > MAX_RECORDS
        || records
            .keys()
            .enumerate()
            .any(|(i, key)| key != &Key::Integer(i as i64 + 1))
    {
        return Err(ParseError::InvalidRecords);
    }
    let mut summary = Summary {
        schema: 1,
        record_count: records.len(),
        client_version: None,
        client_build: None,
        client_interface: None,
        observed_character: None,
        observed_realm: None,
        observed_map: None,
        last_record_at: None,
        latest_scoreboard_rows: None,
        raw_winner: None,
        last_capture_phase: None,
        compatibility: Compatibility::UnknownClient,
    };
    for record in records.values() {
        if record.table().is_none() {
            return Err(ParseError::InvalidRecords);
        }
        let kind = record
            .get("kind")
            .and_then(Value::text)
            .ok_or(ParseError::InvalidRecords)?;
        if kind.len() > 80 || kind.chars().any(char::is_control) {
            return Err(ParseError::InvalidRecords);
        }
        summary.last_record_at = record.get("at").and_then(Value::integer);
        let Some(payload) = record.get("payload") else {
            continue;
        };
        if payload.table().is_none() {
            return Err(ParseError::InvalidRecords);
        }
        match kind {
            "environment" => {
                let build = payload.get("build");
                summary.client_version = numeric_text(build.and_then(|v| v.position(1)), true);
                summary.client_build = numeric_text(build.and_then(|v| v.position(2)), false);
                summary.client_interface = numeric_text(build.and_then(|v| v.position(4)), false);
                summary.observed_map =
                    display(payload.get("instance").and_then(|v| v.position(1)), 256);
            }
            "identity" => {
                summary.observed_character =
                    display(payload.get("name").and_then(|v| v.position(1)), 128);
                summary.observed_realm =
                    display(payload.get("realm").and_then(|v| v.position(1)), 128);
            }
            "scoreboard" => {
                summary.latest_scoreboard_rows = payload
                    .get("rows")
                    .and_then(Value::table)
                    .map(BTreeMap::len);
                summary.raw_winner =
                    numeric_text(payload.get("winner").and_then(|v| v.position(1)), false);
            }
            "capture_marker" => summary.last_capture_phase = display(payload.get("phase"), 128),
            _ => {}
        }
    }
    summary.compatibility = match (
        &summary.client_version,
        &summary.client_build,
        &summary.client_interface,
    ) {
        (Some(version), Some(build), Some(interface))
            if version == "1.60.1"
                && interface == "16001"
                && ["70170", "70245", "70291", "70338"].contains(&build.as_str()) =>
        {
            Compatibility::ObservedDiagnosticBuild
        }
        (Some(_), Some(_), Some(_)) => Compatibility::UnsupportedClient,
        _ => Compatibility::UnknownClient,
    };
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wrapped(records: &str) -> String {
        format!("RBGSCompatibilityData={{schema=1,records={{{records}}}}}")
    }
    #[test]
    fn empty_bom_comments_and_escaped_unicode_are_data_only() {
        let data = wrapped(
            r#"{kind="identity",at=123,payload={name={"Andr\195\169",count=1},realm={"Realm",count=1}}}"#,
        );
        let summary = parse(format!("\u{feff}-- header\n{data} -- done").as_bytes()).unwrap();
        assert_eq!(summary.observed_character.as_deref(), Some("André"));
        assert_eq!(summary.last_record_at, Some(123));
        assert_eq!(parse(wrapped("").as_bytes()).unwrap().record_count, 0);
    }
    #[test]
    fn secret_comments_preserve_unknowns_and_zero() {
        let summary = parse(wrapped(r#"{kind="scoreboard",payload={rows={{values={"Private",nil --[[ secret value ]],"0",count=3}}},winner={"0",count=1}}}"#).as_bytes()).unwrap();
        assert_eq!(summary.raw_winner.as_deref(), Some("0"));
        assert_eq!(summary.latest_scoreboard_rows, Some(1));
        assert!(!serde_json::to_string(&summary).unwrap().contains("Private"));
        assert_eq!(parse(wrapped(r#"{kind="scoreboard",payload={rows={},winner={nil --[[ secret value ]],count=1}}}"#).as_bytes()).unwrap().raw_winner, None);
    }
    #[test]
    fn rejects_executable_lua_and_trailing_content() {
        for input in [
            "RBGSCompatibilityData=dofile(\"secret\")",
            "RBGSCompatibilityData={schema=1,records={}};print(1)",
            "RBGSCompatibilityData={schema=1,records={}} OtherData={}",
            "RBGSCompatibilityData={schema=1,records={},x=function() end}",
            "RBGSCompatibilityData={schema=1,records={},x=1+2}",
            "RBGSCompatibilityData={schema=1,records={},x=os.execute(\"x\")}",
        ] {
            assert!(parse(input.as_bytes()).is_err());
        }
    }
    #[test]
    fn rejects_duplicate_sparse_and_invalid_records() {
        assert_eq!(
            parse(b"RBGSCompatibilityData={schema=1,[\"schema\"]=1,records={}}").unwrap_err(),
            ParseError::DuplicateKey
        );
        for records in [
            "[2]={kind=\"event\"}",
            "[true]={kind=\"event\"}",
            "true",
            "{kind=\"event\",payload=\"bad\"}",
            "{kind=\"event\",[1]=nil,nil}",
        ] {
            assert!(parse(wrapped(records).as_bytes()).is_err());
        }
        assert_eq!(
            parse(b"RBGSCompatibilityData={schema=2,records={}}").unwrap_err(),
            ParseError::UnsupportedSchema
        );
    }
    #[test]
    fn rejects_partial_bad_encoding_and_invalid_escapes() {
        for data in [
            b"RBGSCompatibilityData={".as_slice(),
            b"RBGSCompatibilityData={schema=1,records={}} --[[",
            b"\xff",
            b"RBGSCompatibilityData={schema=1,records={},x=\"\\999\"}",
            b"RBGSCompatibilityData={schema=1,records={},x=\"\\u0041\"}",
        ] {
            assert!(parse(data).is_err());
        }
    }
    #[test]
    fn enforces_byte_depth_string_entry_and_record_limits() {
        assert_eq!(
            parse(&vec![b' '; MAX_BYTES + 1]).unwrap_err(),
            ParseError::TooLarge
        );
        let deep = format!(
            "RBGSCompatibilityData={{schema=1,records={{}},x={}0{}}}",
            "{".repeat(32),
            "}".repeat(32)
        );
        assert_eq!(parse(deep.as_bytes()).unwrap_err(), ParseError::TooDeep);
        let text = format!(
            "RBGSCompatibilityData={{schema=1,records={{}},x=\"{}\"}}",
            "a".repeat(MAX_STRING_BYTES + 1)
        );
        assert_eq!(
            parse(text.as_bytes()).unwrap_err(),
            ParseError::StringTooLong
        );
        let entries = format!(
            "RBGSCompatibilityData={{schema=1,records={{}},x={{{}}}}}",
            "0,".repeat(MAX_ITEMS)
        );
        assert_eq!(
            parse(entries.as_bytes()).unwrap_err(),
            ParseError::TooManyItems
        );
        assert_eq!(
            parse(wrapped(&"{kind=\"event\"},".repeat(301)).as_bytes()).unwrap_err(),
            ParseError::InvalidRecords
        );
    }
    fn lua(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Null => "nil".into(),
            serde_json::Value::Array(values) => format!(
                "{{{}}}",
                values.iter().map(lua).collect::<Vec<_>>().join(",")
            ),
            serde_json::Value::Object(values) => format!(
                "{{{}}}",
                values
                    .iter()
                    .map(|(key, value)| {
                        let key = if let Ok(key) = key.parse::<i64>() {
                            key.to_string()
                        } else {
                            serde_json::to_string(key).unwrap()
                        };
                        format!("[{key}]={}", lua(value))
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            _ => value.to_string(),
        }
    }
    #[test]
    fn parses_real_sanitized_darkspear_fixtures() {
        for (fixture, winner, build) in [
            (
                include_str!("../../../tools/fixtures/darkspear-70291-final.sanitized.json"),
                "0",
                "70291",
            ),
            (
                include_str!(
                    "../../../tools/fixtures/darkspear-70291-alliance-final.sanitized.json"
                ),
                "1",
                "70291",
            ),
            (
                include_str!(
                    "../../../tools/fixtures/darkspear-70338-alliance-final.sanitized.json"
                ),
                "1",
                "70338",
            ),
        ] {
            let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
            let data =
                lua(&serde_json::json!({"schema":fixture["schema"],"records":fixture["records"]}));
            let summary = parse(format!("RBGSCompatibilityData={data}").as_bytes()).unwrap();
            assert_eq!(summary.client_build.as_deref(), Some(build));
            assert_eq!(summary.raw_winner.as_deref(), Some(winner));
            assert_eq!(summary.latest_scoreboard_rows, Some(30));
            assert_eq!(
                summary.compatibility,
                Compatibility::ObservedDiagnosticBuild
            );
        }
    }
    #[test]
    fn parses_full_bounded_history_and_marks_unknown_builds() {
        let row = "{values={\"Synthetic\",\"0\",count=2}}";
        let record =
            "{kind=\"scoreboard\",payload={rows={".to_owned() + &vec![row; 40].join(",") + "}}}";
        assert_eq!(
            parse(wrapped(&vec![record; 300].join(",")).as_bytes())
                .unwrap()
                .record_count,
            300
        );
        let summary = parse(
            wrapped(
                r#"{kind="environment",payload={build={"1.60.1","99999","date","16001",count=4}}}"#,
            )
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(summary.compatibility, Compatibility::UnsupportedClient);
    }
}
