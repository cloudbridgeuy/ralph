#!/usr/bin/env bash
#
# run-loop.sh - Iterative LLM development loop
#
# This is the bash prototype of `ralph run`. Use it to bootstrap
# the implementation of the actual command.
#
# Usage:
#   ./scripts/run-loop.sh [iterations]
#
# If iterations is not provided, defaults to the number of pending
# stories in the PRD file.

set -e

# Configuration (override with environment variables)
DESIGN_FILE="${RALPH_DESIGN_FILE:-.local/designs/2026-01-06-run-command-architecture.md}"
PRD_FILE="${RALPH_PRD_FILE:-.local/plans/prd.toml}"
PROGRESS_FILE="${RALPH_PROGRESS_FILE:-.local/plans/progress.txt}"
COMPLETION_MARKER="${RALPH_COMPLETION_MARKER:-<promise>COMPLETE</promise>}"
LINT_COMMAND="${RALPH_LINT_COMMAND:-cargo xtask lint}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Print colored message
info() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Count pending stories in PRD (stories where passes = false)
count_pending() {
	grep -c 'passes = false' "$PRD_FILE" 2>/dev/null || echo "0"
}

# Check if PRD file exists
if [[ ! -f "$PRD_FILE" ]]; then
	error "PRD file not found at $PRD_FILE"
	echo "Create a PRD file with user stories to begin."
	exit 1
fi

# Touch context files if missing
if [[ ! -f "$DESIGN_FILE" ]]; then
	mkdir -p "$(dirname "$DESIGN_FILE")"
	touch "$DESIGN_FILE"
	warn "Created empty design file: $DESIGN_FILE"
fi

if [[ ! -f "$PROGRESS_FILE" ]]; then
	mkdir -p "$(dirname "$PROGRESS_FILE")"
	touch "$PROGRESS_FILE"
	warn "Created empty progress file: $PROGRESS_FILE"
fi

# Determine iteration count
PENDING=$(count_pending)
if [[ -z "$1" ]]; then
	ITERATIONS="$PENDING"
	info "No iteration count provided, defaulting to pending stories: $ITERATIONS"
else
	ITERATIONS="$1"
fi

if [[ "$ITERATIONS" -le 0 ]]; then
	success "No pending stories. Nothing to do."
	exit 0
fi

info "Starting run loop with up to $ITERATIONS iterations"
info "PRD: $PRD_FILE ($PENDING pending stories)"
info "Design: $DESIGN_FILE"
info "Progress: $PROGRESS_FILE"
echo "----------------------------"

for ((i = 1; i <= ITERATIONS; i++)); do
	# Pre-check: any pending stories?
	PENDING=$(count_pending)
	if [[ "$PENDING" -eq 0 ]]; then
		success "All stories complete. Exiting loop."
		break
	fi

	echo ""
	info "Iteration $i of $ITERATIONS ($PENDING pending stories)"
	echo "----------------------------"

	# Snapshot PRD before iteration
	PRD_BEFORE=$(cat "$PRD_FILE")

	# Build the prompt
	PROMPT="@$DESIGN_FILE @$PRD_FILE @$PROGRESS_FILE

1. Find the highest-priority feature to work on and work on that feature.
   This should be the one YOU decide has the highest priority - not necessarily the first in the list.

2. Check that the '$LINT_COMMAND' command passes successfully.
   You can't mark a user story as complete if this command fails.
   Even when the issue is not related to your current changes.

3. Update the PRD with the work that was done by setting passes = true for completed stories.

4. Append your progress to the progress.txt file.
   Use this to leave a note for the next person working in the codebase.

5. Make a git commit of that feature without Claude attribution.

6. If you find some PRD is missing in order to complete or extend the task you are working on, you may append it to the PRD using the appropriate format.

ONLY WORK ON A SINGLE FEATURE.

IF YOU NOTICE A FILE GOING OVER 1000 LINES CONSIDER UPDATING IT INTO A MODULE OR MOVING THE TESTS TO A DIFFERENT FILE, AND USE THE #[path = ...] PATTERN.

If, while implementing the feature, you notice all stories in the PRD are complete, output $COMPLETION_MARKER."

	# Run claude directly (no capture, pure streaming)
	set +e
	claude --permission-mode acceptEdits -p "$PROMPT" --allowedTools "Read,Edit,Bash" --output-format stream-json --verbose | jq
	exit_code=$?
	set -e

	if [[ "$exit_code" -ne 0 ]]; then
		error "Claude exited with non-zero status (exit code: $exit_code)"
		echo ""
		read -p "Retry this iteration? [y/N] " -n 1 -r
		echo ""
		if [[ $REPLY =~ ^[Yy]$ ]]; then
			((i--)) # Decrement to retry this iteration
			continue
		else
			error "Aborting."
			exit 1
		fi
	fi

	# Post-check: did PRD change?
	PRD_AFTER=$(cat "$PRD_FILE")
	if [[ "$PRD_BEFORE" == "$PRD_AFTER" ]]; then
		error "PRD unchanged after iteration. LLM may be stuck."
		warn "Check $PROGRESS_FILE for notes."
		exit 1
	fi

	# Post-check: any pending stories left?
	PENDING=$(count_pending)
	if [[ "$PENDING" -eq 0 ]]; then
		success "All stories complete after iteration $i!"
		break
	fi

	success "Iteration $i complete. $PENDING stories remaining."
done

echo ""
echo "----------------------------"
FINAL_PENDING=$(count_pending)
if [[ "$FINAL_PENDING" -eq 0 ]]; then
	success "Run complete. All stories implemented!"
else
	info "Run complete. $FINAL_PENDING stories still pending."
fi
