# Isolated insights acceptance fixture

This unexecuted fixture uses the existing `gitops/local/compose.yaml` with a
job-local overlay. It builds the checked-out API, messages, gateway and voice
images, starts only their dependencies, and runs a Node client inside the private
compose network. All published ports are removed. A unique compose project name,
fresh keys and job-owned volumes isolate it from other local stacks. The cleanup
trap removes only that project's containers, volumes, generated environment file
and the image tags created for this job.

The client creates two disposable accounts and a guild, joins through an invite,
tests widget access, sends a Standard guild message, delivers and replays a
crosspost while checking one additional marker, joins voice, sets status as
the connected member, checks `CHANNEL_UPDATE`, leaves voice and the guild, and
reads the resulting counts. It explicitly expects message and voice coverage to
remain incomplete. It neither uses production nor asserts completeness for
historical or crash-gap data.

The workflow is dormant unless the draft PR title contains `[insights-e2e]`.
The workflow currently requires a private-repository checkout secret that is not
available; do not arm it. In an approved isolated VM window, use `run.sh` with
pre-existing, verified pinned source directories through `QA_API_DIR`,
`QA_MESSAGES_DIR`, `QA_GATEWAY_DIR`, `QA_VOICE_DIR`, and `QA_GITOPS_DIR` instead;
no credentials need to be copied. Set `GITHUB_WORKSPACE` to the disposable parent
of the `gitops` checkout and numeric `GITHUB_RUN_ID` and `GITHUB_RUN_ATTEMPT`
for the isolated compose project. The workflow pins
the companion source commits; update and review those pins when their PR heads
change. Report the Actions run ID and exact commits with the result. A passing
fixture covers a known clean input, not a rollout or real voice media.

Estimated cold run: 30–60 minutes, five image builds plus roughly 3–5 GiB of
running-service memory and potentially 10–20 GiB of build cache. Warm caches may
reduce this. Docker Compose must support the `!reset` merge tag. No run has been
performed yet.
