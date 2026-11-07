# Commits

PRs into `main` land as squash commits through GitHub's merge queue, so the PR title becomes the commit subject.

- Use conventional-commit subjects (`feat(ts): ...`, `fix: ...`, `chore: ...`, `docs: ...`).
- AI commit attribution goes in a `Co-Authored-By:` trailer, not the commit body.
- Never commit binaries or build artifacts.

# PRs

Keep the body short and structured, not narrated.
Have at least these sections:

- **Problem**: a summary of the problem and why this PR is needed.
- **Approach**: a summary of the approach taken to solve the problem.
- **Impact**: a bullet point for every public API change.
- **Alternatives**: any alternative approaches considered.
- **Follow-ups**: any issues encountered or quests created.

Never rebase or force-push a pushed branch; merge the base branch in instead.
Open a draft PR while working, run `just ci` before every push, and mark the PR ready yourself when the work is done.

# Landing

PRs land on `main` through GitHub's merge queue, one PR at a time, as squash commits.
Queue the reviewed head with `quest merge <pr> --land <sha>` (`/quest-merge` does this).
The required `Queue` check skips on pull requests; the merge group runs `just ci` against the merge group's base on the self-hosted runner.
That build lives in `.github/workflows/queue-build.yml`, which the merge group calls from `main`, so a PR's edit to it runs only after the PR lands.
The host's `quest land watch` daemon runs the Codex fixer when the queue drops a PR for a failed build or a conflict, and re-enqueues healthy heads at the back of the queue.
A fix that needs a maintainer decision is handed back with a diagnosis on the PR.
The admin bypass of `main`'s ruleset is for emergencies.
`just ci` selects what the diff touches; `just gate` runs the full suite nightly.

# Reviews

Review each PR locally with `/quest-review`, as `quest context` describes; merging never waits on an online reviewer, and never request one.
The PR body's `## Review` record counts as the required review when it names the head being merged, and so does a recorded skip of a change too trivial to review.
Address online reviewers' findings already posted on that head before merging: fix them, or reply where you disagree.

# Follow-ups

Record out-of-scope findings as follow-up quests with `/quest-plan`, not GitHub issues.

# Forks

A hard fork never merges upstream: fixes are cherry-picked by hand, and each carried change is listed in the fork's README with its upstream PR or the reason it has none.
Each imported component keeps its own license and records its upstream source and revision.
