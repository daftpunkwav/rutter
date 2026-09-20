# rutter-dashboard/ — supervision dashboard

English | [中文](README.zh.md)

Local web server (127.0.0.1, per-launch token) streaming events and
screencast frames to the embedded vanilla-JS frontend, and submitting
human approval decisions back to the broker.

## Boundary

Observation plus verdict submission — the dashboard never executes
actions (blueprint §5). It reads the event backbone (replay, then
live), pulls screencast frames only while a viewer watches, and posts
decisions to the same broker the sessions park on. Every endpoint goes
through one gate: loopback `Host` check + token (query or HttpOnly
cookie) — see `src/auth.rs`.

## Consumers

- `cli` attaches it to `serve` with `--dashboard PORT`.
