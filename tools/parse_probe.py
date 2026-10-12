"""Read the diagnostic addon's restricted SavedVariables format without executing Lua."""

import argparse
from collections import Counter
import json
import re
from pathlib import Path


TOKEN = re.compile(
    r"\s+|--\[\[(?:[^\]]|\](?!\]))*\]\]|--[^\n]*|\[|\]|\{|\}|=|,|;|"
    r'"(?:\\.|[^"\\])*"|'
    r"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?|[A-Za-z_][A-Za-z_0-9]*"
)
MAX_BYTES = 2_000_000
MAX_DEPTH = 30
MAX_ITEMS = 200_000


def tokenize(source):
    tokens = []
    position = 0
    while position < len(source):
        match = TOKEN.match(source, position)
        if not match:
            raise ValueError(f"Unsupported Lua syntax at offset {position}")
        value = match.group()
        position = match.end()
        if not value.isspace() and not value.startswith("--"):
            tokens.append(value)
        if len(tokens) > MAX_ITEMS * 8:
            raise ValueError("Too many tokens")
    return tokens


def decode_string(token):
    result = []
    index = 1
    while index < len(token) - 1:
        char = token[index]
        if char != "\\":
            result.append(char)
            index += 1
            continue
        index += 1
        if index >= len(token) - 1:
            raise ValueError("Incomplete string escape")
        escape = token[index]
        if escape in '"\\':
            result.append(escape)
            index += 1
        elif escape == "n":
            result.append("\n")
            index += 1
        elif escape == "r":
            result.append("\r")
            index += 1
        elif escape == "t":
            result.append("\t")
            index += 1
        elif escape.isdigit():
            digits = token[index:index + 3]
            match = re.match(r"[0-9]{1,3}", digits)
            result.append(chr(int(match.group())))
            index += len(match.group())
        else:
            raise ValueError(f"Unsupported string escape: {escape}")
    return "".join(result)


class Parser:
    def __init__(self, tokens):
        self.tokens = tokens
        self.index = 0
        self.items = 0

    def peek(self):
        return self.tokens[self.index] if self.index < len(self.tokens) else None

    def take(self, expected=None):
        token = self.peek()
        if token is None or (expected is not None and token != expected):
            raise ValueError(f"Expected {expected or 'token'}, got {token}")
        self.index += 1
        return token

    def value(self, depth=0):
        if depth > MAX_DEPTH:
            raise ValueError("SavedVariables nesting is too deep")
        token = self.take()
        if token == "{":
            entries = {}
            next_index = 1
            while self.peek() != "}":
                if self.peek() is None:
                    raise ValueError("Unclosed table")
                if self.peek() == "[":
                    self.take("[")
                    key = self.value(depth + 1)
                    self.take("]")
                    self.take("=")
                elif self.index + 1 < len(self.tokens) and self.tokens[self.index + 1] == "=":
                    key = self.take()
                    self.take("=")
                else:
                    key = next_index
                    next_index += 1
                if not isinstance(key, (str, int)) or isinstance(key, bool):
                    raise ValueError("Unsupported table key")
                if key in entries:
                    raise ValueError("Duplicate table key")
                entries[key] = self.value(depth + 1)
                self.items += 1
                if self.items > MAX_ITEMS:
                    raise ValueError("Too many table entries")
                if self.peek() in (",", ";"):
                    self.take()
                elif self.peek() != "}":
                    raise ValueError("Expected table separator")
            self.take("}")
            if entries and all(isinstance(key, int) for key in entries):
                if set(entries) == set(range(1, len(entries) + 1)):
                    return [entries[index] for index in range(1, len(entries) + 1)]
            return {str(key): value for key, value in entries.items()}
        if token.startswith('"'):
            return decode_string(token)
        if token == "true":
            return True
        if token == "false":
            return False
        if token == "nil":
            return None
        if re.fullmatch(r"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?", token):
            return float(token) if "." in token else int(token)
        raise ValueError(f"Executable or unsupported value: {token}")


def parse(source):
    if len(source.encode("utf-8")) > MAX_BYTES:
        raise ValueError("SavedVariables file is too large")
    parser = Parser(tokenize(source))
    parser.take("RBGSCompatibilityData")
    parser.take("=")
    result = parser.value()
    if parser.peek() is not None:
        raise ValueError("Unexpected content after SavedVariables table")
    if not isinstance(result, dict) or result.get("schema") != 1:
        raise ValueError("Unexpected diagnostic schema")
    if result.get("records") == {}:
        result["records"] = []
    if not isinstance(result.get("records"), list):
        raise ValueError("Missing diagnostic records")
    return result


def summarize(data):
    records = data["records"]
    kinds = Counter(record.get("kind") for record in records if isinstance(record, dict))
    events = Counter()
    restored = []
    failures = 0
    capabilities = {}
    registrations = {}
    scoreboard_counts = []
    identity_guid_present = []
    identity_guids = []
    build_info = {}
    for record in records:
        if not isinstance(record, dict):
            continue
        payload = record.get("payload")
        if not isinstance(payload, dict):
            continue
        if record.get("kind") == "event" and isinstance(payload.get("name"), str):
            events[payload["name"]] += 1
        if record.get("kind") == "session_restored":
            restored.append(payload.get("existingRecords"))
        if record.get("kind") == "capabilities":
            capabilities = {key: value for key, value in payload.items() if isinstance(value, str)}
        if record.get("kind") == "event_registration":
            registrations = {key: value == "registered" for key, value in payload.items()}
        if record.get("kind") == "scoreboard":
            scoreboard_counts.append(payload.get("count"))
        if record.get("kind") == "identity":
            guid = payload.get("guid")
            identity_guid_present.append(isinstance(guid, dict) and bool(guid.get("1")))
            if isinstance(guid, dict) and isinstance(guid.get("1"), str):
                identity_guids.append(guid["1"])
        if record.get("kind") == "environment":
            build = payload.get("build")
            if isinstance(build, dict):
                def numeric_text(value):
                    return value if isinstance(value, str) and re.fullmatch(r"[0-9.]+", value) else None
                build_info = {
                    "version": numeric_text(build.get("1")),
                    "build": numeric_text(build.get("2")),
                    "interface": numeric_text(build.get("4")),
                }
        failures += sum(1 for key, value in payload.items() if key.endswith("Error") and value)
    return {
        "schema": data["schema"],
        "record_count": len(records),
        "record_kinds": dict(sorted(kinds.items())),
        "event_counts": dict(sorted(events.items())),
        "restored_record_counts": restored,
        "api_call_failures": failures,
        "api_types": dict(sorted(capabilities.items())),
        "registered_events": dict(sorted(registrations.items())),
        "scoreboard_counts": scoreboard_counts,
        "identity_guid_present": identity_guid_present,
        "identity_guid_consistent": len(set(identity_guids)) == 1 if identity_guids else None,
        "build_info": build_info,
    }


def main():
    command = argparse.ArgumentParser(description=__doc__)
    command.add_argument("saved_variables", type=Path)
    command.add_argument("--output", type=Path, help="Optional local JSON output path")
    command.add_argument("--summary", action="store_true", help="Print counts without character identifiers")
    args = command.parse_args()
    raw = args.saved_variables.read_text(encoding="utf-8-sig")
    result = parse(raw)
    encoded = json.dumps(summarize(result) if args.summary else result, indent=2, ensure_ascii=False)
    if args.output:
        args.output.write_text(encoded + "\n", encoding="utf-8")
    else:
        print(encoded)


if __name__ == "__main__":
    main()
