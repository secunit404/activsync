# Changelog

## [1.2.3](https://github.com/secunit404/activsync/compare/v1.2.2...v1.2.3) (2026-07-17)


### Bug Fixes

* improve settings feedback and activity dialog UI ([#30](https://github.com/secunit404/activsync/issues/30)) ([e4fccb8](https://github.com/secunit404/activsync/commit/e4fccb8dd9f46f04fa3bf13da81fdb3c964a5f81))


### Code Refactoring

* activities UI refresh — selection, manual sync, connections ([#32](https://github.com/secunit404/activsync/issues/32)) ([88026bd](https://github.com/secunit404/activsync/commit/88026bddf8700756082ecebf8722041a08e65a3b))

## [1.2.2](https://github.com/secunit404/activsync/compare/v1.2.1...v1.2.2) (2026-07-16)


### Bug Fixes

* end the Strava rate-limit pause at the quota reset, not before it ([#25](https://github.com/secunit404/activsync/issues/25)) ([17372eb](https://github.com/secunit404/activsync/commit/17372eb7f2c45e0c2abe33f5bbfdf68f466d7f07))
* quieten routine log noise (uvicorn "error" label, health probes, static assets) ([#26](https://github.com/secunit404/activsync/issues/26)) ([4d5ea68](https://github.com/secunit404/activsync/commit/4d5ea683df7fa98998b3090d4806986e6296926f))
* restore dev mock parity with the batched Strava fetch ([#27](https://github.com/secunit404/activsync/issues/27)) ([986768f](https://github.com/secunit404/activsync/commit/986768f1fcf489f611a6e18c3e88cd230e21f5fa))
* serve /favicon.ico so browsers stop 404ing on every visit ([#28](https://github.com/secunit404/activsync/issues/28)) ([20e99de](https://github.com/secunit404/activsync/commit/20e99debc948d4e18a3aadf8a17cc51300a7111b))

## [1.2.1](https://github.com/secunit404/activsync/compare/v1.2.0...v1.2.1) (2026-07-16)


### Bug Fixes

* stop exhausting Strava's rate limit on every poll cycle ([#23](https://github.com/secunit404/activsync/issues/23)) ([116ed37](https://github.com/secunit404/activsync/commit/116ed37a7e7845c70b471b855793c253c4341964))

## [1.2.0](https://github.com/secunit404/activsync/compare/v1.1.2...v1.2.0) (2026-07-16)


### Features

* rework the mobile activity row and move the stylesheet into static/css ([#21](https://github.com/secunit404/activsync/issues/21)) ([f1276b8](https://github.com/secunit404/activsync/commit/f1276b839ea6a504aff7f69d849cbc3e87517172))

## [1.1.2](https://github.com/secunit404/activsync/compare/v1.1.1...v1.1.2) (2026-07-16)


### Bug Fixes

* correct mobile layout issues in settings and activity detail ([#19](https://github.com/secunit404/activsync/issues/19)) ([4dbda62](https://github.com/secunit404/activsync/commit/4dbda62a679badc1c47dfd1bfa1c1e464b8a15d2))

## [1.1.1](https://github.com/secunit404/activsync/compare/v1.1.0...v1.1.1) (2026-07-16)


### Bug Fixes

* report Strava OAuth and runtime failures instead of 500s and silence ([#16](https://github.com/secunit404/activsync/issues/16)) ([54c03b1](https://github.com/secunit404/activsync/commit/54c03b11247c0cd039461b02acae3bfb2e8444f9))


### Documentation

* trim the callback domain section to what the wizard can't say ([#18](https://github.com/secunit404/activsync/issues/18)) ([6b82d61](https://github.com/secunit404/activsync/commit/6b82d610376f45530a634bf7efe3f58ee73508ec))

## [1.1.0](https://github.com/secunit404/activsync/compare/v1.0.0...v1.1.0) (2026-07-15)


### Features

* show app version, update indicator, and GitHub link in a footer ([#14](https://github.com/secunit404/activsync/issues/14)) ([f52737c](https://github.com/secunit404/activsync/commit/f52737c8865c4a29e9366c784f178849bcdd0261))

## [1.0.0](https://github.com/secunit404/activsync/compare/activsync-v0.1.0...activsync-v1.0.0) (2026-07-15)


### Features

* initial public release of ActivSync ([0859d8b](https://github.com/secunit404/activsync/commit/0859d8b3b0a4e7678e469ec8e71db47d0ab05ec5))


### Documentation

* rewrite README for v1 (badges, Docker quick-start, config) ([debc5b1](https://github.com/secunit404/activsync/commit/debc5b12ade049eea47a07f2cec276ce872e5ac3))


### Miscellaneous

* release 1.0.0 ([99e3ebe](https://github.com/secunit404/activsync/commit/99e3ebe6fd9f577232f81c2bcbc76de94c4a7263))
