import unittest

from parse_probe import parse, summarize


class ParseProbeTests(unittest.TestCase):
    def test_parses_saved_variable_table(self):
        source = '''RBGSCompatibilityData = {
          ["schema"] = 1,
          ["records"] = {
            { ["at"] = 123, ["kind"] = "event", ["payload"] = { ["name"] = "PVP_MATCH_COMPLETE" } },
          },
        }'''
        result = parse(source)
        self.assertEqual(result["records"][0]["payload"]["name"], "PVP_MATCH_COMPLETE")

    def test_rejects_executable_lua(self):
        with self.assertRaises(ValueError):
            parse('RBGSCompatibilityData = dofile("secret")')

    def test_rejects_trailing_code(self):
        with self.assertRaises(ValueError):
            parse('RBGSCompatibilityData = {schema=1,records={}}; print("oops")')

    def test_empty_records_and_escaped_values(self):
        self.assertEqual(parse('RBGSCompatibilityData = {schema=1,records={}}')["records"], [])
        source = 'RBGSCompatibilityData = {schema=1,records={{kind="a\\n\\034b"}}}'
        self.assertEqual(parse(source)["records"][0]["kind"], 'a\n"b')

    def test_rejects_duplicate_keys(self):
        with self.assertRaises(ValueError):
            parse('RBGSCompatibilityData = {schema=1,schema=2,records={}}')

    def test_summary_does_not_expose_identifiers(self):
        data = {"schema": 1, "records": [
            {"kind": "identity", "payload": {"name": ["PrivateCharacter"]}},
            {"kind": "session_restored", "payload": {"existingRecords": 1}},
        ]}
        result = summarize(data)
        self.assertEqual(result["restored_record_counts"], [1])
        self.assertNotIn("PrivateCharacter", str(result))

    def test_parses_secret_value_comments_without_losing_separators(self):
        source = '''RBGSCompatibilityData = {schema=1,records={
          {kind="scoreboard",payload={rows={{values={
            "Synthetic", nil --[[ secret value ]],
            nil --[[ secret value ]], "0", count=4
          }}}}}
        }}'''
        values = parse(source)["records"][0]["payload"]["rows"][0]["values"]
        self.assertEqual(values, {"1": "Synthetic", "2": None, "3": None, "4": "0", "count": 4})

    def test_parses_bounded_full_scoreboard_history(self):
        row = '{values={"Synthetic",' + ','.join('"0"' for _ in range(23)) + ',count=24}}'
        record = '{kind="scoreboard",payload={count=20,rows={' + ','.join([row]*20) + '}}}'
        source = 'RBGSCompatibilityData={schema=1,records={' + ','.join([record]*300) + '}}'
        self.assertEqual(len(parse(source)["records"]), 300)

    def test_rejects_entry_limit_overflow(self):
        from unittest.mock import patch
        with patch("parse_probe.MAX_ITEMS", 3):
            with self.assertRaisesRegex(ValueError, "Too many"):
                parse('RBGSCompatibilityData={schema=1,records={{kind="event",payload={name="test"}}}}')


if __name__ == "__main__":
    unittest.main()
