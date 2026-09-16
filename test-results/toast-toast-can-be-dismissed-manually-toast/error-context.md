# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: toast.spec.ts >> toast can be dismissed manually
- Location: e2e/toast.spec.ts:55:1

# Error details

```
Test timeout of 30000ms exceeded.
```

```
Error: locator.click: Test timeout of 30000ms exceeded.
Call log:
  - waiting for getByTestId('bulk-action-bar').getByRole('button', { name: /exclude/i })
    - locator resolved to <button data-slot="button" data-size="default" data-variant="outline" class="group/button inline-flex shrink-0 items-center justify-center rounded-lg border bg-clip-padding text-sm font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 active:not-aria-[haspopup]:translate-y-px disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 [&_sv…>Exclude 1</button>
  - attempting click action
    2 × waiting for element to be visible, enabled and stable
      - element is not enabled
    - retrying click action
    - waiting 20ms
    2 × waiting for element to be visible, enabled and stable
      - element is not enabled
    - retrying click action
      - waiting 100ms
    58 × waiting for element to be visible, enabled and stable
       - element is not enabled
     - retrying click action
       - waiting 500ms

```

# Page snapshot

```yaml
- generic [ref=e1]:
  - generic [ref=e2]:
    - generic:
      - generic:
        - img
    - navigation "Primary" [ref=e3]:
      - img "ActivSync" [ref=e4]
      - list [ref=e5]:
        - listitem [ref=e6]:
          - link "Activities" [ref=e7] [cursor=pointer]:
            - /url: /
            - img [ref=e8]
        - listitem [ref=e10]:
          - link "Hevy" [ref=e11] [cursor=pointer]:
            - /url: /hevy
            - img [ref=e12]
        - listitem [ref=e18]:
          - link "Settings" [ref=e19] [cursor=pointer]:
            - /url: /settings
            - img [ref=e20]
      - generic [ref=e23]:
        - link "View source on GitHub" [ref=e24] [cursor=pointer]:
          - /url: https://github.com/secunit404/activsync
          - img [ref=e25]
        - generic [ref=e28]: v1.2.4
    - generic [ref=e30]:
      - banner [ref=e31]:
        - generic [ref=e32]:
          - generic "ActivSync" [ref=e33]
          - heading "Activities" [level=1] [ref=e34]
          - paragraph [ref=e35]: Review your Garmin sync history before it reaches Strava.
      - region "Activity totals" [ref=e36]:
        - generic [ref=e37]:
          - generic [ref=e38]:
            - generic [ref=e39]: PENDING
            - generic [ref=e40]: "14"
          - generic [ref=e41]:
            - generic [ref=e42]: HELD
            - generic [ref=e43]: "7"
          - generic [ref=e44]:
            - generic [ref=e45]: PUBLISHED
            - generic [ref=e46]: "9"
          - generic [ref=e47]:
            - generic [ref=e48]: THIS WEEK
            - generic [ref=e49]: 1h 00m
      - generic [ref=e50]:
        - group "Filter activities by status" [ref=e52]:
          - button "All39" [pressed] [ref=e53] [cursor=pointer]
          - button "Pending14" [ref=e54] [cursor=pointer]
          - button "Held7" [ref=e55] [cursor=pointer]
          - button "Published9" [ref=e56] [cursor=pointer]
          - button "Missing2" [ref=e57] [cursor=pointer]
        - generic [ref=e58]:
          - combobox "Sort activities" [ref=e59]:
            - option "Newest first" [selected]
            - option "Oldest first"
          - img
      - generic [ref=e61]:
        - generic [ref=e62]: 1 selected
        - generic [ref=e63]:
          - button "Clear" [ref=e64] [cursor=pointer]
          - button "Exclude 0" [disabled]
          - button "Publish 0" [disabled]
      - table [ref=e66]:
        - rowgroup [ref=e67]:
          - row "Select all activities ACTIVITY TYPE TIME DISTANCE EFFORT STATUS Open" [ref=e68]:
            - columnheader "Select all activities" [ref=e69]:
              - checkbox "Select all activities" [checked=mixed] [ref=e70] [cursor=pointer]:
                - generic:
                  - img
            - columnheader "ACTIVITY" [ref=e71]
            - columnheader "TYPE" [ref=e72]
            - columnheader "TIME" [ref=e73]
            - columnheader "DISTANCE" [ref=e74]
            - columnheader "EFFORT" [ref=e75]
            - columnheader "STATUS" [ref=e76]
            - columnheader "Open" [ref=e77]:
              - generic [ref=e78]: Open
        - rowgroup [ref=e79]:
          - row "Select Strength (watch) Strength (watch) 27 Jul · 19:04 STRENGTH TRAINING 1h 00m 00s — 128 bpm EXCLUDED" [ref=e80] [cursor=pointer]:
            - cell "Select Strength (watch)" [ref=e81]:
              - checkbox "Select Strength (watch)" [checked] [active] [ref=e82]:
                - generic:
                  - img
            - cell "Strength (watch) 27 Jul · 19:04" [ref=e83]:
              - generic [ref=e84]:
                - link "Strength (watch)" [ref=e85]:
                  - /url: /910002
                - generic [ref=e86]: 27 Jul · 19:04
            - cell "STRENGTH TRAINING" [ref=e87]:
              - generic "strength_training" [ref=e88]: STRENGTH TRAINING
            - cell "1h 00m 00s" [ref=e89]
            - cell "—" [ref=e90]
            - cell "128 bpm" [ref=e91]
            - cell "EXCLUDED" [ref=e92]:
              - generic [ref=e93]: EXCLUDED
            - cell [ref=e94]:
              - img [ref=e95]
          - row "Select Gym session (watch) Gym session (watch) 27 Jul · 17:34 · via Hevy STRENGTH TRAINING 1h 00m 00s — 128 bpm PENDING" [ref=e97] [cursor=pointer]:
            - cell "Select Gym session (watch)" [ref=e98]:
              - checkbox "Select Gym session (watch)" [ref=e99]
            - cell "Gym session (watch) 27 Jul · 17:34 · via Hevy" [ref=e100]:
              - generic [ref=e101]:
                - link "Gym session (watch)" [ref=e102]:
                  - /url: /910001
                - generic [ref=e103]: 27 Jul · 17:34 · via Hevy
            - cell "STRENGTH TRAINING" [ref=e104]:
              - generic "strength_training" [ref=e105]: STRENGTH TRAINING
            - cell "1h 00m 00s" [ref=e106]
            - cell "—" [ref=e107]
            - cell "128 bpm" [ref=e108]
            - cell "PENDING" [ref=e109]:
              - generic [ref=e110]: PENDING
            - cell [ref=e111]:
              - img [ref=e112]
          - row "Select Forgot the watch (Hevy) Forgot the watch (Hevy) 26 Jul · 17:34 · via Hevy STRENGTH TRAINING 1h 00m 00s — 128 bpm PENDING" [ref=e114] [cursor=pointer]:
            - cell "Select Forgot the watch (Hevy)" [ref=e115]:
              - checkbox "Select Forgot the watch (Hevy)" [ref=e116]
            - cell "Forgot the watch (Hevy) 26 Jul · 17:34 · via Hevy" [ref=e117]:
              - generic [ref=e118]:
                - link "Forgot the watch (Hevy)" [ref=e119]:
                  - /url: /910050
                - generic [ref=e120]: 26 Jul · 17:34 · via Hevy
            - cell "STRENGTH TRAINING" [ref=e121]:
              - generic "strength_training" [ref=e122]: STRENGTH TRAINING
            - cell "1h 00m 00s" [ref=e123]
            - cell "—" [ref=e124]
            - cell "128 bpm" [ref=e125]
            - cell "PENDING" [ref=e126]:
              - generic [ref=e127]: PENDING
            - cell [ref=e128]:
              - img [ref=e129]
          - 'row "Select Strength workout #3 Strength workout #3 21 Jul · 15:34 STRENGTH TRAINING 54m 00s — 140 bpm MISSING" [ref=e131] [cursor=pointer]':
            - 'cell "Select Strength workout #3" [ref=e132]':
              - 'checkbox "Select Strength workout #3" [ref=e133]'
            - 'cell "Strength workout #3 21 Jul · 15:34" [ref=e134]':
              - generic [ref=e135]:
                - 'link "Strength workout #3" [ref=e136]':
                  - /url: /900003
                - generic [ref=e137]: 21 Jul · 15:34
            - cell "STRENGTH TRAINING" [ref=e138]:
              - generic "strength_training" [ref=e139]: STRENGTH TRAINING
            - cell "54m 00s" [ref=e140]
            - cell "—" [ref=e141]
            - cell "140 bpm" [ref=e142]
            - cell "MISSING" [ref=e143]:
              - generic [ref=e144]: MISSING
            - cell [ref=e145]:
              - img [ref=e146]
          - 'row "Select Saturday ride #2 Saturday ride #2 13 Jul · 12:34 CYCLING 2h 08m 00s 52.40 km 141 bpm PUBLISHED" [ref=e148] [cursor=pointer]':
            - 'cell "Select Saturday ride #2" [ref=e149]':
              - 'checkbox "Select Saturday ride #2" [ref=e150]'
            - 'cell "Saturday ride #2 13 Jul · 12:34" [ref=e151]':
              - generic [ref=e152]:
                - 'link "Saturday ride #2" [ref=e153]':
                  - /url: /900002
                - generic [ref=e154]: 13 Jul · 12:34
            - cell "CYCLING" [ref=e155]:
              - generic "cycling" [ref=e156]: CYCLING
            - cell "2h 08m 00s" [ref=e157]
            - cell "52.40 km" [ref=e158]
            - cell "141 bpm" [ref=e159]
            - cell "PUBLISHED" [ref=e160]:
              - generic [ref=e161]: PUBLISHED
            - cell [ref=e162]:
              - img [ref=e163]
          - 'row "Select Morning run #1 Morning run #1 5 Jul · 09:34 RUNNING 42m 00s 7.85 km 142 bpm PENDING" [ref=e165] [cursor=pointer]':
            - 'cell "Select Morning run #1" [ref=e166]':
              - 'checkbox "Select Morning run #1" [ref=e167]'
            - 'cell "Morning run #1 5 Jul · 09:34" [ref=e168]':
              - generic [ref=e169]:
                - 'link "Morning run #1" [ref=e170]':
                  - /url: /900001
                - generic [ref=e171]: 5 Jul · 09:34
            - cell "RUNNING" [ref=e172]:
              - generic "running" [ref=e173]: RUNNING
            - cell "42m 00s" [ref=e174]
            - cell "7.85 km" [ref=e175]
            - cell "142 bpm" [ref=e176]
            - cell "PENDING" [ref=e177]:
              - generic [ref=e178]: PENDING
            - cell [ref=e179]:
              - img [ref=e180]
          - 'row "Select Mountain hike #3 Mountain hike #3 21 Jun · 15:34 HIKING 4h 12m 00s 14.60 km 137 bpm HELD" [ref=e182] [cursor=pointer]':
            - 'cell "Select Mountain hike #3" [ref=e183]':
              - 'checkbox "Select Mountain hike #3" [ref=e184]'
            - 'cell "Mountain hike #3 21 Jun · 15:34" [ref=e185]':
              - generic [ref=e186]:
                - 'link "Mountain hike #3" [ref=e187]':
                  - /url: /900006
                - generic [ref=e188]: 21 Jun · 15:34
            - cell "HIKING" [ref=e189]:
              - generic "hiking" [ref=e190]: HIKING
            - cell "4h 12m 00s" [ref=e191]
            - cell "14.60 km" [ref=e192]
            - cell "137 bpm" [ref=e193]
            - cell "HELD" [ref=e194]:
              - generic [ref=e195]: HELD
            - cell [ref=e196]:
              - img [ref=e197]
          - 'row "Select Forest trail run #2 Forest trail run #2 13 Jun · 12:34 TRAIL RUNNING 1h 08m 00s 10.20 km 138 bpm PENDING" [ref=e199] [cursor=pointer]':
            - 'cell "Select Forest trail run #2" [ref=e200]':
              - 'checkbox "Select Forest trail run #2" [ref=e201]'
            - 'cell "Forest trail run #2 13 Jun · 12:34" [ref=e202]':
              - generic [ref=e203]:
                - 'link "Forest trail run #2" [ref=e204]':
                  - /url: /900005
                - generic [ref=e205]: 13 Jun · 12:34
            - cell "TRAIL RUNNING" [ref=e206]:
              - generic "trail_running" [ref=e207]: TRAIL RUNNING
            - cell "1h 08m 00s" [ref=e208]
            - cell "10.20 km" [ref=e209]
            - cell "138 bpm" [ref=e210]
            - cell "PENDING" [ref=e211]:
              - generic [ref=e212]: PENDING
            - cell [ref=e213]:
              - img [ref=e214]
          - 'row "Select Evening walk #1 Evening walk #1 5 Jun · 09:34 WALKING 38m 00s 3.12 km 139 bpm EXCLUDED" [ref=e216] [cursor=pointer]':
            - 'cell "Select Evening walk #1" [ref=e217]':
              - 'checkbox "Select Evening walk #1" [ref=e218]'
            - 'cell "Evening walk #1 5 Jun · 09:34" [ref=e219]':
              - generic [ref=e220]:
                - 'link "Evening walk #1" [ref=e221]':
                  - /url: /900004
                - generic [ref=e222]: 5 Jun · 09:34
            - cell "WALKING" [ref=e223]:
              - generic "walking" [ref=e224]: WALKING
            - cell "38m 00s" [ref=e225]
            - cell "3.12 km" [ref=e226]
            - cell "139 bpm" [ref=e227]
            - cell "EXCLUDED" [ref=e228]:
              - generic [ref=e229]: EXCLUDED
            - cell [ref=e230]:
              - img [ref=e231]
          - 'row "Select Yoga recovery #3 Yoga recovery #3 21 May · 15:34 YOGA 36m 00s — 142 bpm EXCLUDED" [ref=e233] [cursor=pointer]':
            - 'cell "Select Yoga recovery #3" [ref=e234]':
              - 'checkbox "Select Yoga recovery #3" [ref=e235]'
            - 'cell "Yoga recovery #3 21 May · 15:34" [ref=e236]':
              - generic [ref=e237]:
                - 'link "Yoga recovery #3" [ref=e238]':
                  - /url: /900009
                - generic [ref=e239]: 21 May · 15:34
            - cell "YOGA" [ref=e240]:
              - generic "yoga" [ref=e241]: YOGA
            - cell "36m 00s" [ref=e242]
            - cell "—" [ref=e243]
            - cell "142 bpm" [ref=e244]
            - cell "EXCLUDED" [ref=e245]:
              - generic [ref=e246]: EXCLUDED
            - cell [ref=e247]:
              - img [ref=e248]
          - 'row "Select Indoor bike #2 Indoor bike #2 13 May · 12:34 INDOOR CYCLING 51m 00s 28.70 km 135 bpm PENDING" [ref=e250] [cursor=pointer]':
            - 'cell "Select Indoor bike #2" [ref=e251]':
              - 'checkbox "Select Indoor bike #2" [ref=e252]'
            - 'cell "Indoor bike #2 13 May · 12:34" [ref=e253]':
              - generic [ref=e254]:
                - 'link "Indoor bike #2" [ref=e255]':
                  - /url: /900008
                - generic [ref=e256]: 13 May · 12:34
            - cell "INDOOR CYCLING" [ref=e257]:
              - generic "indoor_cycling" [ref=e258]: INDOOR CYCLING
            - cell "51m 00s" [ref=e259]
            - cell "28.70 km" [ref=e260]
            - cell "135 bpm" [ref=e261]
            - cell "PENDING" [ref=e262]:
              - generic [ref=e263]: PENDING
            - cell [ref=e264]:
              - img [ref=e265]
          - 'row "Select Pool intervals #1 Pool intervals #1 5 May · 09:34 SWIMMING 47m 00s — 136 bpm MISSING" [ref=e267] [cursor=pointer]':
            - 'cell "Select Pool intervals #1" [ref=e268]':
              - 'checkbox "Select Pool intervals #1" [ref=e269]'
            - 'cell "Pool intervals #1 5 May · 09:34" [ref=e270]':
              - generic [ref=e271]:
                - 'link "Pool intervals #1" [ref=e272]':
                  - /url: /900007
                - generic [ref=e273]: 5 May · 09:34
            - cell "SWIMMING" [ref=e274]:
              - generic "swimming" [ref=e275]: SWIMMING
            - cell "47m 00s" [ref=e276]
            - cell "—" [ref=e277]
            - cell "136 bpm" [ref=e278]
            - cell "MISSING" [ref=e279]:
              - generic [ref=e280]: MISSING
            - cell [ref=e281]:
              - img [ref=e282]
          - 'row "Select Cardio workout #3 Cardio workout #3 21 Apr · 15:34 CARDIO 39m 00s — 139 bpm PUBLISHED" [ref=e284] [cursor=pointer]':
            - 'cell "Select Cardio workout #3" [ref=e285]':
              - 'checkbox "Select Cardio workout #3" [ref=e286]'
            - 'cell "Cardio workout #3 21 Apr · 15:34" [ref=e287]':
              - generic [ref=e288]:
                - 'link "Cardio workout #3" [ref=e289]':
                  - /url: /900012
                - generic [ref=e290]: 21 Apr · 15:34
            - cell "CARDIO" [ref=e291]:
              - generic "cardio" [ref=e292]: CARDIO
            - cell "39m 00s" [ref=e293]
            - cell "—" [ref=e294]
            - cell "139 bpm" [ref=e295]
            - cell "PUBLISHED" [ref=e296]:
              - generic [ref=e297]: PUBLISHED
            - cell [ref=e298]:
              - img [ref=e299]
          - 'row "Select Cross trainer #2 Cross trainer #2 13 Apr · 12:34 ELLIPTICAL 44m 00s — 140 bpm HELD" [ref=e301] [cursor=pointer]':
            - 'cell "Select Cross trainer #2" [ref=e302]':
              - 'checkbox "Select Cross trainer #2" [ref=e303]'
            - 'cell "Cross trainer #2 13 Apr · 12:34" [ref=e304]':
              - generic [ref=e305]:
                - 'link "Cross trainer #2" [ref=e306]':
                  - /url: /900011
                - generic [ref=e307]: 13 Apr · 12:34
            - cell "ELLIPTICAL" [ref=e308]:
              - generic "elliptical" [ref=e309]: ELLIPTICAL
            - cell "44m 00s" [ref=e310]
            - cell "—" [ref=e311]
            - cell "140 bpm" [ref=e312]
            - cell "HELD" [ref=e313]:
              - generic [ref=e314]: HELD
            - cell [ref=e315]:
              - img [ref=e316]
          - 'row "Select Rowing intervals #1 Rowing intervals #1 5 Apr · 09:34 ROWING 32m 00s 6.40 km 141 bpm PENDING" [ref=e318] [cursor=pointer]':
            - 'cell "Select Rowing intervals #1" [ref=e319]':
              - 'checkbox "Select Rowing intervals #1" [ref=e320]'
            - 'cell "Rowing intervals #1 5 Apr · 09:34" [ref=e321]':
              - generic [ref=e322]:
                - 'link "Rowing intervals #1" [ref=e323]':
                  - /url: /900010
                - generic [ref=e324]: 5 Apr · 09:34
            - cell "ROWING" [ref=e325]:
              - generic "rowing" [ref=e326]: ROWING
            - cell "32m 00s" [ref=e327]
            - cell "6.40 km" [ref=e328]
            - cell "141 bpm" [ref=e329]
            - cell "PENDING" [ref=e330]:
              - generic [ref=e331]: PENDING
            - cell [ref=e332]:
              - img [ref=e333]
          - 'row "Select Saturday ride #3 Saturday ride #3 21 Mar · 14:34 CYCLING 2h 08m 00s 52.40 km 136 bpm HELD" [ref=e335] [cursor=pointer]':
            - 'cell "Select Saturday ride #3" [ref=e336]':
              - 'checkbox "Select Saturday ride #3" [ref=e337]'
            - 'cell "Saturday ride #3 21 Mar · 14:34" [ref=e338]':
              - generic [ref=e339]:
                - 'link "Saturday ride #3" [ref=e340]':
                  - /url: /900015
                - generic [ref=e341]: 21 Mar · 14:34
            - cell "CYCLING" [ref=e342]:
              - generic "cycling" [ref=e343]: CYCLING
            - cell "2h 08m 00s" [ref=e344]
            - cell "52.40 km" [ref=e345]
            - cell "136 bpm" [ref=e346]
            - cell "HELD" [ref=e347]:
              - generic [ref=e348]: HELD
            - cell [ref=e349]:
              - img [ref=e350]
          - 'row "Select Morning run #2 Morning run #2 13 Mar · 11:34 RUNNING 42m 00s 7.85 km 137 bpm PENDING" [ref=e352] [cursor=pointer]':
            - 'cell "Select Morning run #2" [ref=e353]':
              - 'checkbox "Select Morning run #2" [ref=e354]'
            - 'cell "Morning run #2 13 Mar · 11:34" [ref=e355]':
              - generic [ref=e356]:
                - 'link "Morning run #2" [ref=e357]':
                  - /url: /900014
                - generic [ref=e358]: 13 Mar · 11:34
            - cell "RUNNING" [ref=e359]:
              - generic "running" [ref=e360]: RUNNING
            - cell "42m 00s" [ref=e361]
            - cell "7.85 km" [ref=e362]
            - cell "137 bpm" [ref=e363]
            - cell "PENDING" [ref=e364]:
              - generic [ref=e365]: PENDING
            - cell [ref=e366]:
              - img [ref=e367]
          - 'row "Select Backcountry day #1 Backcountry day #1 5 Mar · 08:34 BACKCOUNTRY SKIING SNOWBOARDING WS 5h 20m 00s 9.40 km 138 bpm PUBLISHED" [ref=e369] [cursor=pointer]':
            - 'cell "Select Backcountry day #1" [ref=e370]':
              - 'checkbox "Select Backcountry day #1" [ref=e371]'
            - 'cell "Backcountry day #1 5 Mar · 08:34" [ref=e372]':
              - generic [ref=e373]:
                - 'link "Backcountry day #1" [ref=e374]':
                  - /url: /900013
                - generic [ref=e375]: 5 Mar · 08:34
            - cell "BACKCOUNTRY SKIING SNOWBOARDING WS" [ref=e376]:
              - generic "backcountry_skiing_snowboarding_ws" [ref=e377]: BACKCOUNTRY SKIING SNOWBOARDING WS
            - cell "5h 20m 00s" [ref=e378]
            - cell "9.40 km" [ref=e379]
            - cell "138 bpm" [ref=e380]
            - cell "PUBLISHED" [ref=e381]:
              - generic [ref=e382]: PUBLISHED
            - cell [ref=e383]:
              - img [ref=e384]
          - 'row "Select Forest trail run #3 Forest trail run #3 21 Feb · 14:34 TRAIL RUNNING 1h 08m 00s 10.20 km 141 bpm PENDING" [ref=e386] [cursor=pointer]':
            - 'cell "Select Forest trail run #3" [ref=e387]':
              - 'checkbox "Select Forest trail run #3" [ref=e388]'
            - 'cell "Forest trail run #3 21 Feb · 14:34" [ref=e389]':
              - generic [ref=e390]:
                - 'link "Forest trail run #3" [ref=e391]':
                  - /url: /900018
                - generic [ref=e392]: 21 Feb · 14:34
            - cell "TRAIL RUNNING" [ref=e393]:
              - generic "trail_running" [ref=e394]: TRAIL RUNNING
            - cell "1h 08m 00s" [ref=e395]
            - cell "10.20 km" [ref=e396]
            - cell "141 bpm" [ref=e397]
            - cell "PENDING" [ref=e398]:
              - generic [ref=e399]: PENDING
            - cell [ref=e400]:
              - img [ref=e401]
          - 'row "Select Evening walk #2 Evening walk #2 13 Feb · 11:34 WALKING 38m 00s 3.12 km 142 bpm EXCLUDED" [ref=e403] [cursor=pointer]':
            - 'cell "Select Evening walk #2" [ref=e404]':
              - 'checkbox "Select Evening walk #2" [ref=e405]'
            - 'cell "Evening walk #2 13 Feb · 11:34" [ref=e406]':
              - generic [ref=e407]:
                - 'link "Evening walk #2" [ref=e408]':
                  - /url: /900017
                - generic [ref=e409]: 13 Feb · 11:34
            - cell "WALKING" [ref=e410]:
              - generic "walking" [ref=e411]: WALKING
            - cell "38m 00s" [ref=e412]
            - cell "3.12 km" [ref=e413]
            - cell "142 bpm" [ref=e414]
            - cell "EXCLUDED" [ref=e415]:
              - generic [ref=e416]: EXCLUDED
            - cell [ref=e417]:
              - img [ref=e418]
      - generic [ref=e420]:
        - generic [ref=e421]: Page 1 of 2
        - navigation "pagination" [ref=e422]:
          - list [ref=e423]:
            - listitem [ref=e424]:
              - link "Go to next page" [ref=e425] [cursor=pointer]:
                - /url: "#"
                - generic [ref=e426]: Next
                - img
  - status "Notifications alt+T"
```

# Test source

```ts
  1  | import { test, expect } from "./fixtures";
  2  | 
  3  | // The brief's spec drives this via the bulk Publish button, but the e2e
  4  | // dev-mock seed (`dev_seed.py`) never sets `strava_tokens` — Strava is
  5  | // always "disconnected" in this environment (see `view.connection_status`),
  6  | // so `BulkActionBar`'s Publish button is permanently `publishDisabled` here,
  7  | // independent of which row is selected. Faking a Strava connection for e2e
  8  | // is out of scope for a toast restyle, so this exercises the same
  9  | // `useActivityActions` success-toast pipeline through Exclude instead — the
  10 | // one bulk action that is never gated on a connection (see
  11 | // `bulk-action-bar.tsx`'s `publishDisabled` doc comment).
  12 | function bulkActionBar(page: import("@playwright/test").Page) {
  13 |   return page.getByTestId("bulk-action-bar");
  14 | }
  15 | 
  16 | // `role="status"` alone is also ambiguous: `ui/spinner.tsx` already puts
  17 | // `role="status"` on every in-flight spinner (see its `aria-label="Loading"`),
  18 | // and the bulk action bar's own (disabled) Publish button shows one for as
  19 | // long as the shared `useActivityActions` mutation this Exclude click drives
  20 | // is pending. The toaster's root `<section>` is the one with sonner's
  21 | // default `containerAriaLabel`, "Notifications" — name on that to get past
  22 | // the spinner.
  23 | function toastRegion(page: import("@playwright/test").Page) {
  24 |   return page.getByRole("status", { name: /notifications/i });
  25 | }
  26 | 
  27 | // Tick a row that can actually be excluded, rather than "the first
  28 | // checkbox". Every project in this suite shares one server and DB, and the
  29 | // first test here excludes a row — so by the second test that row is already
  30 | // excluded, and Exclude is (correctly) disabled for it. Selecting a PENDING
  31 | // row keeps each test independent of what ran before it.
  32 | //
  33 | // This used to pass by accident: Exclude was enabled regardless of status,
  34 | // the second click 409'd, and an *error* toast satisfied assertions that
  35 | // only check a toast appeared and can be closed.
  36 | async function selectPendingRow(page: import("@playwright/test").Page) {
  37 |   // `:visible` because a CSS locator, unlike a role query, would otherwise
  38 |   // match the display:none copy of the layout this viewport hides.
  39 |   const row = page
  40 |     .locator('tr:visible, [data-testid="activities-cards"] > div:visible')
  41 |     .filter({ hasText: "PENDING" })
  42 |     .first();
  43 |   await row.getByRole("checkbox").check();
  44 |   await expect(
  45 |     bulkActionBar(page).getByRole("button", { name: /^Exclude [1-9]/ }),
  46 |   ).toBeEnabled();
  47 | }
  48 | 
  49 | test("excluding an activity shows a success toast", async ({ page }) => {
  50 |   await selectPendingRow(page);
  51 |   await bulkActionBar(page).getByRole("button", { name: /exclude/i }).click();
  52 |   await expect(toastRegion(page)).toContainText(/exclude/i);
  53 | });
  54 | 
  55 | test("toast can be dismissed manually", async ({ page }) => {
  56 |   await selectPendingRow(page);
> 57 |   await bulkActionBar(page).getByRole("button", { name: /exclude/i }).click();
     |                                                                       ^ Error: locator.click: Test timeout of 30000ms exceeded.
  58 |   await page.getByRole("button", { name: /close/i }).click();
  59 |   await expect(toastRegion(page)).toBeHidden();
  60 | });
  61 | 
```