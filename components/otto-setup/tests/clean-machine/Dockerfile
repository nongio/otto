# otto-setup on a machine that has nothing yet: a clean Debian-family image
# with only otto-setup and otto-agents copied in. A successful `docker build`
# is a passing test; scripts/test-otto-setup.sh stages the context (the two
# binaries under bin/, Otto's plugin under plugins/, the two files beside
# this) and runs it.
#
# BASE picks the image. The binaries are the ones built on this machine, so
# the image's glibc has to be at least as new as the symbols they use; the
# first RUN as the user says so when it is not.
ARG BASE=ubuntu:24.04
FROM ${BASE}

ENV DEBIAN_FRONTEND=noninteractive

# python3 drives the screens; ca-certificates lets apt fetch Node.js below.
RUN apt-get update -qq && apt-get install -qq -y --no-install-recommends \
        ca-certificates python3 >/dev/null \
    && rm -rf /var/lib/apt/lists/*

COPY bin/otto-setup bin/otto-agents /usr/bin/
COPY plugins/ /usr/share/otto/plugins/otto/
COPY check.sh screen.py /usr/local/lib/otto-setup-test/

# An ordinary user with a home of their own, as on a desktop.
RUN useradd --create-home tester
USER tester
WORKDIR /home/tester
ENV HOME=/home/tester \
    PATH=/home/tester/.local/bin:/usr/local/bin:/usr/bin:/bin

# A loader error here ("version `GLIBC_2.xx' not found") means the image is
# older than the machine that built the binaries: pick a newer BASE.
RUN otto-setup --version && otto-agents --version

# 1. Nothing installed.
RUN bash /usr/local/lib/otto-setup-test/check.sh fresh

# 2. The distribution's own Node.js, which is what `sudo apt install
#    nodejs npm` gives: too old for the Claude Code adapter on every
#    Debian and Ubuntu release so far.
USER root
RUN apt-get update -qq && apt-get install -qq -y --no-install-recommends nodejs npm >/dev/null \
    && rm -rf /var/lib/apt/lists/* && node --version
USER tester
RUN bash /usr/local/lib/otto-setup-test/check.sh old-node

# 3. An agent that is ready: --apply has to write it down.
RUN bash /usr/local/lib/otto-setup-test/check.sh apply
