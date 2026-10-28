"""Tests for the parts of probe.py that guard against leaking account details or hitting the wrong process.

Run: uv run --no-project --with psutil python -B -m unittest discover -s probes
"""

import unittest
import unittest.mock
from pathlib import Path

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


class SessionShowsTests(unittest.TestCase):
    def session_that_printed(self, output: bytes) -> probe.Session:
        launched = unittest.mock.Mock(output=bytearray(output))
        return probe.Session(pid=1, config_dir=Path("c"), record_path=Path("c/sessions/1.json"), _launched=launched)

    def test_finds_text_drawn_with_colours_and_cursor_moves(self) -> None:
        session = self.session_that_printed(b"\x1b[2G\x1b[38;5;211m> draft\x1b[39m\x1b[1Cthat was typed\r\n")

        self.assertTrue(session.shows("draft that was typed"))

    def test_does_not_find_text_that_was_never_drawn(self) -> None:
        self.assertFalse(self.session_that_printed(b"> \r\n").shows("draft"))


class IntegrationGuardTests(unittest.TestCase):
    """The integration check stops every idle session it finds, so it must never run on a developer machine."""

    def test_integration_refuses_to_run_outside_github_actions(self) -> None:
        with (
            unittest.mock.patch.dict("os.environ", {}, clear=True),
            unittest.mock.patch.object(probe, "start_session") as start_session,
            unittest.mock.patch.object(probe.subprocess, "run") as run,
        ):
            result = probe.integration(Path("claude-reaper"))

        self.assertEqual(result, 2)
        start_session.assert_not_called()
        run.assert_not_called()

    def test_a_case_that_raises_is_a_failed_check_and_the_next_case_still_runs(self) -> None:
        with (
            unittest.mock.patch.dict("os.environ", {"GITHUB_ACTIONS": "true"}),
            unittest.mock.patch.object(probe, "claude_version", return_value="2.1.310 (Claude Code)"),
            unittest.mock.patch.object(probe, "start_session", side_effect=RuntimeError("node is not on PATH")) as start_session,
            unittest.mock.patch.object(probe, "report") as report,
        ):
            result = probe.integration(Path("claude-reaper"))

        self.assertEqual(result, 1)
        self.assertEqual(start_session.call_count, 2)
        observations = report.call_args_list[0].args[2]
        self.assertIn("RuntimeError: node is not on PATH", observations["case completed"])


class IntegrationCasesTests(unittest.TestCase):
    def test_windows_integration_covers_console_and_piped_sessions(self) -> None:
        self.assertEqual([case[1] for case in probe.integration_cases("win32")], ["piped", "console"])

    def test_unix_integration_covers_terminal_and_piped_sessions(self) -> None:
        for platform in ("linux", "darwin"):
            with self.subTest(platform=platform):
                self.assertEqual([case[1] for case in probe.integration_cases(platform)], ["piped", "pty"])


if __name__ == "__main__":
    unittest.main()
