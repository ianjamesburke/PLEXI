"""Offline queue contract tests."""
import importlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from contextlib import nullcontext
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
queue = importlib.import_module("merge_queue")
lock = importlib.import_module("alpha_lock")


class QueueTests(unittest.TestCase):
    def test_blocks(self):
        self.assertEqual(queue.block("Candidate filters:\n- app::\n- cli::\n\n- ignored", "Candidate filters"), ("app::", "cli::"))
        self.assertEqual(queue.block("Candidate filters:\n- app::\nprose\n- ignored", "Candidate filters"), ("app::",))
        for body in ("absent", "Candidate filters:\n\n- app::"):
            with self.assertRaises(queue.Refusal):
                queue.block(body, "Candidate filters")

    def test_required_filters(self):
        for path, needed in (("src/host/plugin_server.rs", "testing::socket_policy_tests"),
                             ("src/cli/mod.rs", "testing::socket_policy_tests"),
                             ("src/cli/mod.rs", "testing::permission_gate_tests::"),
                             ("src/ui/foo.rs", "ui::"),
                             ("src/app/quit.rs", "ui_tests::tests::")):
            with self.subTest(path=path):
                self.assertIn(needed, queue.required_filters([path]))
                with self.assertRaises(queue.Refusal):
                    queue.validate_filters([path], ("app::",))

    def test_batches(self):
        self.assertEqual(list(queue.batches([9, 5, 8, 1, 4, 7])), [[9, 5, 8, 1], [4, 7]])

    def test_admission_skips_one_pr_without_reordering_the_batch(self):
        with tempfile.TemporaryDirectory() as tmp:
            train = queue.Queue(Path(tmp), Path(tmp), Mock())
            train.report = Mock()
            train.alpha = Mock(return_value="base")
            train.admit = Mock(side_effect=[
                queue.PR(3, "first", "a", (), (), ()), queue.Refusal("draft"),
                queue.PR(2, "third", "b", (), (), ()),
            ])
            train.attempt = Mock(return_value="pass")
            train.execute([3, 5, 2, 3])
            self.assertEqual(train.skipped, {5: "draft"})
            self.assertEqual([pr.number for pr in train.attempt.call_args.args[0]], [3, 2])

    def test_split_and_retry(self):
        calls = []
        def run(group):
            calls.append(tuple(group))
            if len(calls) == 1:
                return "infra"
            return "fail" if 2 in group else "pass"
        park = Mock()
        queue.qualify([1, 2, 3], run, park, Mock())
        self.assertEqual(calls, [(1, 2, 3), (1, 2, 3), (1,), (2, 3), (2,), (3,)])
        park.assert_called_once_with(2)
        run = Mock(return_value="infra")
        with self.assertRaises(queue.Refusal):
            queue.qualify([1, 2], run, Mock(), Mock())
        self.assertEqual(run.call_count, 2)
        run = Mock(return_value="pass")
        queue.qualify([3, 1, 2], run, Mock(), Mock())
        run.assert_called_once_with([3, 1, 2])

    def test_landing_guards_and_requeue(self):
        for scenario in ("rebase tree", "base", "post tree", "state", "ancestor", "refused", "ok"):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as tmp:
                runner = Mock()
                train = queue.Queue(Path(tmp), Path(tmp), runner)
                merge = "a" * 40
                train.alpha = Mock(side_effect=[
                    "wrong" if scenario == "base" else "base", "base",
                    "base" if scenario == "refused" else merge, "moved",
                ])
                train.push = Mock(return_value="new-head")
                train.rebase = Mock(return_value=("new-head", "wrong" if scenario == "rebase tree" else "tree"))
                train.report = Mock()
                runner.checked.side_effect = lambda *args, **kwargs: {
                    ("git", "rev-parse", f"{merge}^{{tree}}"): "wrong" if scenario == "post tree" else "tree",
                    ("git", "show", "-s", "--format=%P", merge): "base",
                }[args]
                runner.run.side_effect = lambda *args, **kwargs: subprocess.CompletedProcess(
                    args, int(scenario == ("refused" if args[0] == "just" else "ancestor")), "")
                train.view = Mock(return_value={
                    "state": "OPEN" if scenario in ("state", "refused") else "MERGED",
                    "mergeCommit": {"oid": merge},
                })
                first = queue.PR(1, "first", "old", (), (), ())
                second = queue.PR(2, "second", "old", (), (), ())
                entries = [queue.Entry(first, "prior" if scenario == "rebase tree" else "base", "tip", "tree"),
                           queue.Entry(second, "tip", "second-tip", "second-tree")]
                train.readmit = Mock()
                if scenario == "refused":
                    self.assertEqual(train.land("base", entries), [second])
                else:
                    with self.assertRaises(queue.Refusal):
                        train.land("base", entries)
                self.assertEqual(train.merged, [1] if scenario == "ok" else [])
                if scenario in ("rebase tree", "base"):
                    runner.run.assert_not_called()

    def test_all_pass_lands_both_in_input_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            runner = Mock()
            train = queue.Queue(Path(tmp), Path(tmp), runner)
            train.report = Mock()
            first_merge, second_merge = "a" * 40, "b" * 40
            train.alpha = Mock(side_effect=["base", "base", first_merge,
                                           first_merge, first_merge, second_merge])
            train.push = Mock(side_effect=["tip1", "rebased-tip2"])
            train.rebase = Mock(return_value=("rebased-tip2", "tree2"))
            train.view = Mock(side_effect=[
                {"state": "MERGED", "mergeCommit": {"oid": first_merge}},
                {"state": "MERGED", "mergeCommit": {"oid": second_merge}},
            ])
            runner.checked.side_effect = lambda *args, **kwargs: {
                ("git", "rev-parse", f"{first_merge}^{{tree}}"): "tree1",
                ("git", "rev-parse", f"{second_merge}^{{tree}}"): "tree2",
                ("git", "show", "-s", "--format=%P", first_merge): "base",
                ("git", "show", "-s", "--format=%P", second_merge): first_merge,
            }[args]
            runner.run.return_value = subprocess.CompletedProcess([], 0, "")
            first = queue.PR(8, "first", "old1", (), (), ())
            second = queue.PR(3, "second", "old2", (), (), ())
            entries = [queue.Entry(first, "base", "tip1", "tree1"),
                       queue.Entry(second, "tip1", "tip2", "tree2")]
            train.readmit = Mock()
            self.assertEqual(train.land("base", entries), [])
            self.assertEqual(train.merged, [8, 3])
            self.assertEqual([call.args for call in runner.run.call_args_list if call.args[0] == "just"],
                             [("just", "bs-merge", "8", "--reviewed-head", "tip1"),
                              ("just", "bs-merge", "3", "--reviewed-head", "rebased-tip2")])
            train.rebase.assert_called_once_with("tip2", "tip1", first_merge)
            train.summary([8, 3])
            reports = [call.args[0] for call in train.report.call_args_list]
            self.assertTrue(any("#8" in line and "prefix-untested (end state tested)" in line for line in reports[:-1]))
            self.assertIn("prefix-untested (end state tested)", reports[-1])

    def test_landing_readmits_before_touching_pr_branch(self):
        for change in ("risk label", "linked stint", "filters", "head", "check"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as tmp:
                runner = Mock()
                train = queue.Queue(Path(tmp), Path(tmp), runner)
                head = "a" * 40
                info = dict(headRefName="feature/fixture", headRefOid=head, state="OPEN",
                            isDraft=False, baseRefName="alpha", isCrossRepository=False,
                            labels=[], body="Candidate filters:\n- app::", reviewDecision="APPROVED",
                            reviews=[dict(state="APPROVED", author={"login": "reviewer"},
                                          commit={"oid": head})])
                train.view = Mock(return_value=info)
                train.lane = Mock()
                train.git = Mock(return_value="scripts/merge_queue.py")
                runner.run.return_value = subprocess.CompletedProcess([], 0, "[]")
                first = train.admit(1, "base")
                runner.run.side_effect = lambda *args, **kwargs: (
                    subprocess.CompletedProcess(args, 1, "") if args[0] == "just"
                    else runner.run.return_value)
                if change == "risk label":
                    info["labels"] = [{"name": "risk-high"}]
                elif change == "linked stint":
                    tasks = Path(tmp) / ".stint" / "tasks"
                    tasks.mkdir(parents=True)
                    (tasks / "1234-test.md").write_text("tags: [risk-high]\n")
                    info["body"] += "\n\nStint: 1234"
                elif change == "filters":
                    info["body"] = "Candidate filters:\n- cli::"
                elif change == "head":
                    info["headRefOid"] = "b" * 40
                    info["reviews"][0]["commit"]["oid"] = info["headRefOid"]
                else:
                    runner.run.return_value = subprocess.CompletedProcess([], 1, '[{"bucket":"fail"}]')
                second = queue.PR(2, "second", "old", (), (), ())
                train.alpha = Mock(return_value="base")
                train.push = Mock()
                entries = [queue.Entry(first, "base", "tip", "tree"),
                           queue.Entry(second, "tip", "second-tip", "second-tree")]
                self.assertEqual(train.land("base", entries), [second])
                train.push.assert_not_called()
                self.assertIn(1, train.skipped)
                self.assertFalse(any(call.args[0] == "just" for call in runner.run.call_args_list))

    def test_rebased_head_keeps_original_approval_and_rechecks_metadata(self):
        with tempfile.TemporaryDirectory() as tmp:
            runner = Mock()
            train = queue.Queue(Path(tmp), Path(tmp), runner)
            original, rebased = "a" * 40, "b" * 40
            info = dict(headRefName="feature/fixture", headRefOid=original, state="OPEN",
                        isDraft=False, baseRefName="alpha", isCrossRepository=False,
                        labels=[], body="Candidate filters:\n- app::", reviewDecision="REVIEW_REQUIRED",
                        reviews=[dict(state="APPROVED", author={"login": "reviewer"},
                                      commit={"oid": original})])
            train.view = Mock(return_value=info)
            train.alpha = Mock(return_value="base")
            train.lane = Mock()
            train.git = Mock(return_value="scripts/merge_queue.py")
            runner.run.return_value = subprocess.CompletedProcess([], 0, "[]")
            pr = train.admit(1, "base")

            def push(pr, head):
                info["headRefOid"] = head
                train.readmit(pr, "base", head)
                info["body"] = "Candidate filters:\n- cli::"
                return head

            train.push = Mock(side_effect=push)
            self.assertEqual(train.land("base", [queue.Entry(pr, "base", rebased, "tree")]), [])
            self.assertIn("candidate requirements changed", train.skipped[1])
            self.assertFalse(any(call.args[0] == "just" for call in runner.run.call_args_list))

    def test_timeout_after_prefix_reports_and_returns_nonzero(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            runner = Mock()
            train = queue.Queue(root, root, runner)
            merge = "a" * 40
            train.alpha = Mock(side_effect=["base", "base", merge, merge, merge, merge])
            train.push = Mock(side_effect=["tip1", "rebased-tip2"])
            train.readmit = Mock()
            train.rebase = Mock(return_value=("rebased-tip2", "tree2"))
            train.view = Mock(side_effect=[{"state": "MERGED", "mergeCommit": {"oid": merge}},
                                           {"state": "OPEN"}])
            train.git = Mock(side_effect=lambda *args: "base" if args[0] == "show" else "tree1")
            def run(*args, **kwargs):
                if args[:3] == ("just", "bs-merge", "3"):
                    self.assertGreaterEqual(kwargs["timeout"], 60)
                    raise subprocess.TimeoutExpired(args, kwargs["timeout"])
                return subprocess.CompletedProcess(args, 0, "")
            runner.run.side_effect = run
            first = queue.PR(8, "first", "old1", (), (), ())
            second = queue.PR(3, "second", "old2", (), (), ())
            entries = [queue.Entry(first, "base", "tip1", "tree1"),
                       queue.Entry(second, "tip1", "tip2", "tree2")]
            train.execute = Mock(side_effect=lambda _: train.land("base", entries))
            with patch.object(queue, "Queue", return_value=train), patch.object(
                    queue, "lock_path", return_value=root / ".git" / "writer"):
                self.assertEqual(queue.main(["8", "3"]), 1)
            log = (root / "queue.log").read_text()
            self.assertIn(f"ALPHA IS AT AN UNTESTED PREFIX: {merge} after PRs [8]", log)
            self.assertIn("timed out", log)
            self.assertIn("prefix-untested (end state tested)", log)

    def test_candidate_unions_filters_and_distinguishes_infra(self):
        with tempfile.TemporaryDirectory() as tmp:
            runner = Mock()
            train = queue.Queue(Path(tmp), Path(tmp), runner)
            train.report = Mock()
            first = queue.PR(8, "first", "a", ("app::", "broker::"), ("pty",), ())
            second = queue.PR(3, "second", "b", ("broker::", "agent::"), ("pty", "other"), ("scene",))
            entries = [queue.Entry(first, "alpha", "tip1", "tree1"),
                       queue.Entry(second, "tip1", "tip2", "tree2")]
            for code, output, expected in (
                (0, "RESULT name=candidate-1 status=pass exit=0 seconds=1 log=/fake", "pass"),
                (1, "merge-queue: candidate gates started\nRESULT name=candidate-2 status=fail exit=1 seconds=1 log=/fake", "fail"),
                (1, "build-cache: floor check failed\nRESULT name=candidate-3 status=fail exit=1 seconds=1 log=/fake", "infra"),
                (1, "wrapper interrupted", "infra"),
            ):
                runner.run.return_value = subprocess.CompletedProcess([], code, output)
                self.assertEqual(train.candidate("alpha", entries), expected)
                env = runner.run.call_args.kwargs["env"]
                self.assertEqual(env["PLEXI_PR_TEST_FILTERS"], "app::\nbroker::\nagent::")
                self.assertEqual(env["PLEXI_PR_REQUIRED_PTY_FILTERS"], "pty\nother")
                self.assertEqual(env["PLEXI_PR_REQUIRED_SCENES"], "scene")
                self.assertEqual(runner.run.call_args.args[-2:], ("alpha", "tip2"))

    def test_attempt_runs_once_for_both_and_requalifies_refused_tail(self):
        with tempfile.TemporaryDirectory() as tmp:
            train = queue.Queue(Path(tmp), Path(tmp), Mock())
            train.alpha = Mock(side_effect=["base", "after-first"])
            first = queue.PR(8, "first", "old1", ("app::",), (), ())
            second = queue.PR(3, "second", "old2", ("cli::",), (), ())
            entries = [queue.Entry(first, "base", "tip1", "tree1"),
                       queue.Entry(second, "tip1", "tip2", "tree2")]
            tail = [queue.Entry(second, "after-first", "tail-tip", "tail-tree")]
            train.stack = Mock(side_effect=[entries, tail])
            train.candidate = Mock(return_value="pass")
            train.land = Mock(side_effect=[[second], []])
            with patch.object(queue, "writer_lock", return_value=nullcontext(3)), patch.object(queue, "lock_path"):
                self.assertEqual(train.attempt([first, second]), "pass")
            self.assertEqual([call.args for call in train.candidate.call_args_list],
                             [("base", entries), ("after-first", tail)])
            self.assertEqual([call.args[:2] for call in train.land.call_args_list],
                             [("base", entries), ("after-first", tail)])

    def test_checks_accept_no_checks_and_pending_and_refuse_failures(self):
        with tempfile.TemporaryDirectory() as tmp:
            runner = Mock()
            train = queue.Queue(Path(tmp), Path(tmp), runner)
            pr = queue.PR(1, "feature/fixture", "pinned", ("app::",), (), ())
            train.view = Mock(return_value={"state": "OPEN", "isDraft": False, "headRefOid": "pinned",
                                            "baseRefName": "alpha", "reviewDecision": "APPROVED"})
            for code, output in ((1, "no checks reported on the 'feature/fixture' branch"),
                                 (8, '[{"bucket":"pending"},{"bucket":"skipping"}]')):
                runner.run.return_value = subprocess.CompletedProcess([], code, output)
                train.check_head(pr, pr.head)
            runner.run.return_value = subprocess.CompletedProcess([], 1, '[{"bucket":"fail"}]')
            with self.assertRaisesRegex(queue.Refusal, "failing"):
                train.check_head(pr, pr.head)
            runner.run.return_value = subprocess.CompletedProcess([], 0, "[]")
            train.view.return_value["headRefOid"] = "moved"
            with self.assertRaisesRegex(queue.Refusal, "pinned head"):
                train.check_head(pr, pr.head)

    def test_lock_contention_and_inheritance(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "writer"
            check = (
                "import sys; sys.path.insert(0, sys.argv[1])\n"
                "from alpha_lock import writer_lock; from pathlib import Path\n"
                "with writer_lock(Path(sys.argv[2]), 0.05): pass"
            )
            cmd = [sys.executable, "-I", "-c", check, str(Path(__file__).resolve().parent), str(path)]
            with lock.writer_lock(path, 0.05) as fd:
                valid = subprocess.run(cmd, pass_fds=(fd,), capture_output=True, text=True)
                self.assertEqual(valid.returncode, 0, valid.stderr)
                forged = subprocess.run(cmd, capture_output=True, text=True)
                self.assertNotEqual(forged.returncode, 0)
                self.assertIn(f"holder pid={os.getpid()}", forged.stderr)
                with path.open("a+") as unrelated:
                    env = {**os.environ, lock.FD: str(unrelated.fileno())}
                    forged = subprocess.run(cmd, env=env, pass_fds=(unrelated.fileno(),),
                                            capture_output=True, text=True)
                self.assertNotEqual(forged.returncode, 0)
                self.assertIn(f"holder pid={os.getpid()}", forged.stderr)


if __name__ == "__main__":
    unittest.main()
