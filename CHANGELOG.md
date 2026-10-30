# Changelog

## [3.2.0](https://github.com/secunit404/activsync/compare/v3.1.0...v3.2.0) (2026-10-30)


### Features

* **hevy:** merging a Hevy workout into Garmin waits a few seconds with an undo ([#105](https://github.com/secunit404/activsync/issues/105)) ([e4b707a](https://github.com/secunit404/activsync/commit/e4b707a33e3ef5aa9851d3d1d22bea9c4290648d))


### Bug Fixes

* **settings:** signing out and removing single sign-on ask first ([#108](https://github.com/secunit404/activsync/issues/108)) ([a7d2d7d](https://github.com/secunit404/activsync/commit/a7d2d7dc82555e49deca85bc80683fd9b3cb1497))
* **settings:** unsaved changes survive background refreshes ([#107](https://github.com/secunit404/activsync/issues/107)) ([33270fc](https://github.com/secunit404/activsync/commit/33270fc848484ef0c9495874bd15ca226e3bea3c))

## [3.1.0](https://github.com/secunit404/activsync/compare/v3.0.0...v3.1.0) (2026-10-30)


### Features

* **activities:** a publish goes through even if you leave the page during the undo ([#103](https://github.com/secunit404/activsync/issues/103)) ([08589bf](https://github.com/secunit404/activsync/commit/08589bfc3c16ae63104e8e242cecdd12482a0fc8))
* **activities:** excluding activities can be undone from the toast ([#102](https://github.com/secunit404/activsync/issues/102)) ([9c43345](https://github.com/secunit404/activsync/commit/9c433450772bc1c9d4133438ad215020714b110b))
* **activities:** held activities say why they're waiting ([#96](https://github.com/secunit404/activsync/issues/96)) ([faa0de8](https://github.com/secunit404/activsync/commit/faa0de85bb26fa41e44561620208a0a6e81b05f4))
* **activities:** open on what needs your review, with one filter row ([#94](https://github.com/secunit404/activsync/issues/94)) ([c9113be](https://github.com/secunit404/activsync/commit/c9113be0b3d9914e5c4e30d2e385d47a29145514))
* **activities:** phones can select the whole page from the selection bar ([#98](https://github.com/secunit404/activsync/issues/98)) ([844e801](https://github.com/secunit404/activsync/commit/844e80123a62dedaf4d70c9ea2bd088ccb86c7dc))
* **activities:** publishing to Strava waits a few seconds with an undo ([#93](https://github.com/secunit404/activsync/issues/93)) ([c3e6ba7](https://github.com/secunit404/activsync/commit/c3e6ba7e78f7616e11867f249f147e252057a290))
* **activities:** the Activities nav shows how many activities need review ([#95](https://github.com/secunit404/activsync/issues/95)) ([481a3d8](https://github.com/secunit404/activsync/commit/481a3d8a51213b4378e26c6dd8d88128d010118e))
* **ui:** page headers drop the small ActivSync label; phones show the logo instead ([#92](https://github.com/secunit404/activsync/issues/92)) ([4bfee8f](https://github.com/secunit404/activsync/commit/4bfee8f8a99fa6b546d1d3d352786688e186bd97))


### Bug Fixes

* **activities:** a blocked Publish says why and links to Reconnect Strava, on phones too ([#101](https://github.com/secunit404/activsync/issues/101)) ([291346b](https://github.com/secunit404/activsync/commit/291346bd5caea2e1aa8bd3e10a6692c969323b52))
* **activities:** activity links read View on Garmin and View on Strava ([#91](https://github.com/secunit404/activsync/issues/91)) ([9669dd1](https://github.com/secunit404/activsync/commit/9669dd1c9103d3a185c8cda375917f9fec6ba126))
* **activities:** even rows, one compact reconnect banner and a clearer subtitle ([#100](https://github.com/secunit404/activsync/issues/100)) ([907532c](https://github.com/secunit404/activsync/commit/907532c17ab1100eb98d9203ca5c39494eccfbe7))
* **activities:** keyboard and screen reader users keep their place ([#99](https://github.com/secunit404/activsync/issues/99)) ([034fd5f](https://github.com/secunit404/activsync/commit/034fd5f068b9c147797d9cf50decc39b79cb7a12))
* **activities:** pagination buttons stay put on the first and last page ([#90](https://github.com/secunit404/activsync/issues/90)) ([1c52b17](https://github.com/secunit404/activsync/commit/1c52b1701194f62d4f82417cba828363005ee2d4))
* **activities:** selecting rows no longer shifts the list, and partial selection shows a dash ([#97](https://github.com/secunit404/activsync/issues/97)) ([cd410c5](https://github.com/secunit404/activsync/commit/cd410c5476eeff18daf876399aab90e56e6653e8))
* **ui:** toast close and action buttons sit cleanly at the right edge ([#89](https://github.com/secunit404/activsync/issues/89)) ([ecdd115](https://github.com/secunit404/activsync/commit/ecdd11598faa0b78a461b83fbe5d0ea195ab5055))

## [3.0.0](https://github.com/secunit404/activsync/compare/v2.1.0...v3.0.0) (2026-10-23)


### ⚠ BREAKING CHANGES

* add authentication for public exposure ([#76](https://github.com/secunit404/activsync/issues/76))

### Features

* add authentication for public exposure ([#76](https://github.com/secunit404/activsync/issues/76)) ([90ba397](https://github.com/secunit404/activsync/commit/90ba3977c09b6247bb4edc1d98d353043a6bb6d1))

## [2.1.0](https://github.com/secunit404/activsync/compare/v2.0.1...v2.1.0) (2026-10-12)


### Features

* upload Hevy strength workouts to Strava with Strava's own exercises ([#72](https://github.com/secunit404/activsync/issues/72)) ([bde7423](https://github.com/secunit404/activsync/commit/bde7423f5dd3309e809f9f045fa434679c5086a0))


### Bug Fixes

* fall back to the FIT when Strava rejects a strength upload ([#74](https://github.com/secunit404/activsync/issues/74)) ([bb17ecd](https://github.com/secunit404/activsync/commit/bb17ecdb27e876fc618117b70978e6fd5beb7645))

## [2.0.1](https://github.com/secunit404/activsync/compare/v2.0.0...v2.0.1) (2026-09-29)


### Performance Improvements

* faster Hevy matching and adaptive Garmin polling ([#67](https://github.com/secunit404/activsync/issues/67)) ([043756b](https://github.com/secunit404/activsync/commit/043756bebbd22aaa3328aec8e559b83e47bb69a5))


### Code Refactoring

* simplify the mobile tab bar to a static glass capsule ([#64](https://github.com/secunit404/activsync/issues/64)) ([dbb783b](https://github.com/secunit404/activsync/commit/dbb783b3bfba5d3def2e831b0b14b94796f68f53))

## [2.0.0](https://github.com/secunit404/activsync/compare/v1.3.0...v2.0.0) (2026-09-23)


### Features

* add Hevy integration and rebuild the frontend in React ([#51](https://github.com/secunit404/activsync/issues/51)) ([ed024bd](https://github.com/secunit404/activsync/commit/ed024bd42f3059fa9bf7e34ef46472804e173824))

## [1.3.0](https://github.com/secunit404/activsync/compare/v1.2.4...v1.3.0) (2026-09-22)


### Features

* report the running build in the footer ([#45](https://github.com/secunit404/activsync/issues/45)) ([32bb91f](https://github.com/secunit404/activsync/commit/32bb91f1e7835111409990a48d2caaec578b1d03))

## [1.2.4](https://github.com/secunit404/activsync/compare/v1.2.3...v1.2.4) (2026-07-17)


### Bug Fixes

* show publish progress in activity actions ([#33](https://github.com/secunit404/activsync/issues/33)) ([312bc7f](https://github.com/secunit404/activsync/commit/312bc7f8cfd83eecdbf10de7c2d3b91d2b6c0ab7))

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
