"""Tests for the parts of probe.py that guard against leaking account details or hitting the wrong process.

Run: uv run --no-project --with psutil python -B -m unittest discover -s probes
"""

import unittest
import unittest.mock

import probe

# Lines that name an account and also use ordinary dialog wording; none may reach a log in strict mode.
ACCOUNT_LINES = [
    "Logged in as Alice Smith. Press Enter to continue",
    "Continue as Alice Smith?",
    "Account Alice Smith does not have access. Please log in again",
    "Acme Trust Ltd",
    "Oxford University Press",
    "│  Christina!  │ Run /terminal-setup to set up terminal integration │",
    "\x1b[3;18HAlice!\x1b[5B\x1b[2GPress Enter to continue",
    "Welcome back\r\x1b[13CAlice!",
]
NAMES = ["Alice", "Smith", "Christina", "Acme", "Oxford"]


class RedactTests(unittest.TestCase):
    def test_welcome_organisation_and_email_lines_are_redacted(self) -> None:
        raw = "Welcome back Jane!\nOrganization: Acme\nuser@example.com logged in\nplain line"

        self.assertEqual(probe.redact(raw, strict=False), "<redacted>\n<redacted> logged in\nplain line")

    def test_cursor_to_column_becomes_spaces_and_colours_vanish(self) -> None:
        raw = "\x1b[2G\x1b[38;5;211mUnable\x1b[9Gto\x1b[12Gconnect\x1b[39m\r\r\n"

        self.assertEqual(probe.redact(raw, strict=False), "Unable to connect")

    def test_blank_and_repeated_lines_collapse(self) -> None:
        self.assertEqual(probe.redact("a\r\n\r\n  \na\nb", strict=False), "a\nb")

    def test_leftover_escape_sequences_do_not_reach_the_output(self) -> None:
        raw = "a\x1b7b\x1b8c\x1b(Bd\x1b=e\x1b>f\x07g"

        self.assertEqual(probe.redact(raw, strict=False), "abcdefg")

    def test_strict_mode_never_prints_screen_text(self) -> None:
        for line in ACCOUNT_LINES:
            with self.subTest(line=line):
                redacted = probe.redact(line, strict=True)

                self.assertTrue(redacted.startswith("<screen text withheld"), redacted)
                for name in NAMES:
                    self.assertNotIn(name, redacted)

    def test_strict_mode_names_only_the_screens_it_recognises(self) -> None:
        raw = "Alice Smith\nDo you trust the files in this folder?\nUnable to connect to Anthropic services"

        self.assertEqual(
            probe.redact(raw, strict=True),
            "<screen text withheld while a token is in use; 3 lines; recognised: folder trust dialog, network error>",
        )

    def test_strict_mode_on_an_unknown_screen_recognises_nothing(self) -> None:
        self.assertEqual(
            probe.redact("Something entirely new", strict=True),
            "<screen text withheld while a token is in use; 1 lines; recognised: nothing>",
        )

    def test_strict_is_the_default_when_a_token_is_in_use(self) -> None:
        with unittest.mock.patch.dict("os.environ", {"CLAUDE_CODE_OAUTH_TOKEN": "x"}):
            self.assertTrue(probe.redact("Alice Smith").startswith("<screen text withheld"))
        with unittest.mock.patch.dict("os.environ", {}, clear=True):
            self.assertEqual(probe.redact("Alice Smith"), "Alice Smith")


class PipedCleanupTests(unittest.TestCase):
    """Cleanup may force-close only the process the launcher started, never a reused PID."""

    def piped(
        self, exit_code: int | None, still_the_same_process: bool
    ) -> tuple[probe._Piped, unittest.mock.Mock, unittest.mock.Mock]:
        process = unittest.mock.Mock()
        process.is_running.return_value = still_the_same_process
        launcher = unittest.mock.Mock()
        launched = object.__new__(probe._Piped)
        launched.pid = 4242
        launched._exit_code = exit_code
        launched._process = process
        launched._launcher = launcher
        return launched, process, launcher

    def test_does_not_touch_the_pid_after_the_session_has_exited(self) -> None:
        launched, process, launcher = self.piped(exit_code=0, still_the_same_process=True)

        launched.force_close()

        process.kill.assert_not_called()
        launcher.kill.assert_called_once()

    def test_does_not_kill_a_pid_that_now_belongs_to_another_process(self) -> None:
        launched, process, _ = self.piped(exit_code=None, still_the_same_process=False)

        launched.force_close()

        process.kill.assert_not_called()

    def test_kills_the_session_it_started(self) -> None:
        launched, process, _ = self.piped(exit_code=None, still_the_same_process=True)

        launched.force_close()

        process.kill.assert_called_once()


if __name__ == "__main__":
    unittest.main()
