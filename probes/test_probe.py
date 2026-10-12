"""Tests for the parts of probe.py that guard against leaking account details.

Run: uv run --no-project --with psutil python -m unittest discover -s probes
"""

import unittest
import unittest.mock

import probe


class RedactTests(unittest.TestCase):
    def test_welcome_organisation_and_email_lines_are_redacted(self) -> None:
        raw = "Welcome back Jane!\nOrganization: Acme\nuser@example.com logged in\nplain line"

        self.assertEqual(probe.redact(raw, strict=False), "<redacted>\n<redacted> logged in\nplain line")

    def test_cursor_to_column_becomes_spaces_and_colours_vanish(self) -> None:
        raw = "\x1b[2G\x1b[38;5;211mUnable\x1b[9Gto\x1b[12Gconnect\x1b[39m\r\r\n"

        self.assertEqual(probe.redact(raw, strict=False), "Unable to connect")

    def test_blank_and_repeated_lines_collapse(self) -> None:
        self.assertEqual(probe.redact("a\r\n\r\n  \na\nb", strict=False), "a\nb")

    def test_strict_mode_keeps_only_known_dialog_wording(self) -> None:
        raw = "Do you trust the files in this folder?\nOpus 4.7 · Claude Max · Alice Smith\nPress Enter to continue"

        self.assertEqual(
            probe.redact(raw, strict=True),
            "Do you trust the files in this folder?\n<redacted>\nPress Enter to continue",
        )

    def test_strict_mode_does_not_keep_a_line_just_because_it_is_a_question(self) -> None:
        self.assertEqual(probe.redact("Are you Alice Smith?", strict=True), "<redacted>")

    def test_strict_mode_still_redacts_identity_lines_that_use_dialog_wording(self) -> None:
        raw = "Welcome, press Enter to continue\nSign in as user@example.com"

        self.assertEqual(probe.redact(raw, strict=True), "<redacted>")

    def test_strict_is_the_default_when_a_token_is_in_use(self) -> None:
        with unittest.mock.patch.dict("os.environ", {"CLAUDE_CODE_OAUTH_TOKEN": "x"}):
            self.assertEqual(probe.redact("Alice Smith"), "<redacted>")
        with unittest.mock.patch.dict("os.environ", {}, clear=True):
            self.assertEqual(probe.redact("Alice Smith"), "Alice Smith")


if __name__ == "__main__":
    unittest.main()
