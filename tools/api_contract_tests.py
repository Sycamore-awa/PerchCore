"""Schema rejection and offline drift checks, separate from stdlib corpus tests."""

import copy
import unittest

from jsonschema import Draft202012Validator
from referencing.exceptions import Unresolvable

from api_catalog import COMMON_ID, REQUEST_ID, RESPONSE_ID
from check_api import check_offline, method_names, normalized, resolve, validators


class ContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.validation = validators()

    def test_complete_catalog_and_examples(self):
        check_offline()
        self.assertEqual(len(method_names()), 35)

    def test_reject_bad_requests(self):
        cases = [
            ("unknown.method", {}), ("version", {"extra": True}), ("version", []),
            ("compatibility", {"extra": True}),
            ("context.open", {"extra": True}),
            ("context.close", {}), ("context.close", {"context": -1}),
            ("txt.open", {}), ("txt.open", {"path": 2}),
            ("txt.slice", {"handle": 0, "chars": 0}),
            ("txt.slice", {"handle": 0, "chars": 20001}),
            ("txt.slice", {"handle": -1, "chars": 1}),
            ("txt.slice", {"handle": 2**64, "chars": 1}),
            ("txt.slice", {"handle": False, "chars": 1}),
            ("txt.slice", {"handle": 0, "chars": 1, "start": "0"}),
            ("txt.analyze", {"bytes_hex": "f"}),
            ("txt.analyze", {"bytes_hex": "0x12"}),
            ("txt.analyze", {"bytes_hex": "中文"}),
            ("txt.inspect", {}), ("txt.inspect", {"path": None}),
            ("txt.inspect", {"path": "a", "bytes_hex": ""}),
            ("txt.inspect", {"path": "a", "encoding": 1}),
            ("book.open", {}), ("jobs.refine", {}),
            ("book.locate", {}),
            ("book.locate", {"handle": 0, "char_start": -1, "char_end": 1}),
            ("book.locate", {"handle": 0, "char_start": 0, "char_end": "1"}),
            ("book.locate", {"handle": 0, "char_start": False, "char_end": 1}),
            ("book.locate", {"handle": 0, "char_start": 0, "char_end": 2**64}),
            ("book.open", {"path": "a", "bytes_hex": ""}),
            ("jobs.refine", {"path": None, "bytes_hex": "00"}),
            ("pdf.analyze", {}), ("pdf.analyze", {"path": "a", "bytes_hex": ""}),
            ("pdf.analyze", {"path": None}),
            ("pipeline.open", {"out_dir": "a", "policy": {"maxAttempts": 65536}}),
            ("pipeline.step", {"handle": 0, "gate": {"allowAny": 1}}),
            ("sync.merge", {"local": None}),
            ("sync.delta", {"since": 23}),
            ("locator.validate", {"href": "a", "perch": {"format": "txt"}}),
            ("locator.validate", {"href": "a", "locations": None}),
            ("locator.validate", {"href": "a", "perch": {"v": 1, "pdfRect": [0, 0, 1]}}),
            ("sync.merge", {"remote": [{"book": "invalid", "hlc": "1-0-a", "op": "tombstone"}]}),
            ("sync.merge", {"remote": [{"book": "00000000-0000-4000-8000-000000000001",
                                        "hlc": "1-0-a", "op": "set"}]}),
        ]
        for method, args in cases:
            with self.subTest(method=method, args=args):
                self.assertFalse(self.validation[REQUEST_ID].is_valid({"method": method, "args": args}))

    def test_structural_schema_keeps_semantic_validation_separate(self):
        for method, args in [
            ("locator.validate", {"href": " ", "locations": {"progression": 2}}),
            ("locator.validate", {"href": "a", "perch": {"v": 1, "format": "txt"}}),
            ("pipeline.open", {"out_dir": "a", "policy": {"maxIndex": 0}}),
            ("hlc.parse", {"hlc": "bad"}),
        ]:
            with self.subTest(method=method):
                self.assertTrue(self.validation[REQUEST_ID].is_valid({"method": method, "args": args}))

    def test_reject_incorrect_responses_and_pipeline_variants(self):
        stats = {"pendingRefine": 0, "pendingIndex": 0, "inFlight": 0, "exhausted": 0}
        cases = [
            ("txt.slice", {"text": "a", "start": 0, "end": 1, "chars": 1}),
            ("txt.offset", {"char": None}),
            ("txt.line", {"line": "0", "start": None, "end": None, "totalLines": 0}),
            ("pipeline.step", {"outcome": "ioFailed", "stats": stats}),
            ("pipeline.step", {"outcome": "failed", "stats": stats, "attempts": 1}),
            ("pipeline.state", {"known": False, "state": "ready"}),
            ("pdf.analyze", {"page_count": 3, "version": "1.7"}),
            ("txt.open", {"error": {"code": "TXT_IO"}}),
            ("version", {"error": {"code": 1, "message": "bad"}}),
            ("compatibility", {"core": "0.1.0", "abi": 1}),
        ]
        for method, result in cases:
            with self.subTest(method=method, result=result):
                self.assertFalse(self.validation[RESPONSE_ID].is_valid({"method": method, "result": result}))

    def test_unknown_schema_reference_cannot_fetch_network(self):
        validator = self.validation[COMMON_ID].evolve(schema={"$ref": "https://example.invalid/schema"})
        with self.assertRaises(Unresolvable):
            validator.validate({})

    def test_source_location_endpoint_units_and_formats_are_explicit(self):
        txt = {"kind": "txt", "charOffset": 9, "byteOffset": 11}
        epub = {"kind": "epub", "href": "/chapter.xhtml", "nodePath": "/2/4/1", "scalarOffset": 5, "utf16Offset": 6}
        def accepted(start, end):
            return self.validation[RESPONSE_ID].is_valid({"method": "book.locate", "result": {
                "start": start, "end": end, "containsSynthetic": False}})
        self.assertTrue(accepted(txt, txt))
        self.assertTrue(accepted(epub, epub))
        self.assertFalse(accepted(txt, epub))
        self.assertFalse(accepted({**txt, "charOffset": -1}, txt))
        self.assertFalse(accepted({**txt, "byteOffset": True}, txt))
        self.assertFalse(accepted({key: value for key, value in epub.items() if key != "utf16Offset"}, epub))
        self.assertFalse(accepted({**epub, "scalarOffset": "5"}, epub))

    def test_capture_nested_values_and_preserve_results(self):
        values = {"open": {"handle": 0, "hits": [{"text": "中文🦀"}]}}
        original = copy.deepcopy(values)
        self.assertEqual(resolve({"h": {"$ref": "open.handle"}, "v": {"$ref": "open.hits.0.text"}}, {}, values),
                         {"h": 0, "v": "中文🦀"})
        self.assertEqual(values, original)

    def test_normalization_preserves_business_reason_and_data(self):
        self.assertEqual(normalized({"error": {"code": "TXT_IO", "message": "platform message"}}, {}),
                         {"error": {"code": "TXT_IO", "message": "<diagnostic>"}})
        self.assertEqual(normalized({"reason": "business error", "chars": 0, "locator": None, "hits": []}, {}),
                         {"reason": "business error", "chars": 0, "locator": None, "hits": []})


if __name__ == "__main__":
    unittest.main()
