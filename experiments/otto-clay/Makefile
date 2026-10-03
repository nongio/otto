# One target per stage; pass extra flags with ARGS, e.g. make motion ARGS="--budget 12 --only s03"
PY ?= python3
ARGS ?=

.PHONY: plan voice transcribe sheet keyframes motion music edit qa animatic all status

plan:
	$(PY) run.py plan $(ARGS) | tee dry-run.log
voice:
	$(PY) run.py voice $(ARGS)
transcribe:
	$(PY) run.py transcribe $(ARGS)
sheet:
	$(PY) run.py sheet $(ARGS)
keyframes:
	$(PY) run.py keyframes $(ARGS)
motion:
	$(PY) run.py motion $(ARGS)
music:
	$(PY) run.py music $(ARGS)
edit:
	$(PY) run.py edit $(ARGS)
qa:
	$(PY) run.py qa $(ARGS)
status:
	$(PY) run.py status $(ARGS)
all:
	$(PY) run.py all $(ARGS)

# Free timing check: espeak-ng scratch narration, placeholder or keyframe shots.
animatic:
	$(PY) run.py voice --scratch
	$(PY) run.py edit --scratch
	$(PY) run.py qa --allow-fail
