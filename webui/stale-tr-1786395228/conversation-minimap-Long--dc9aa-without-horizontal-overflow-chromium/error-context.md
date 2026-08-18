# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: conversation-minimap.spec.ts >> Long conversation history rail >> 769px compact dock preserves a readable chat title without horizontal overflow
- Location: e2e\conversation-minimap.spec.ts:81:3

# Error details

```
Error: expect(received).toBeGreaterThanOrEqual(expected)

Expected: >= 96
Received:    0

Call Log:
- Timeout 5000ms exceeded while waiting on the predicate
```

# Page snapshot

```yaml
- generic [ref=e2]:
  - navigation "Primary" [ref=e3]:
    - generic [ref=e4]:
      - generic [ref=e5]:
        - img [ref=e6]
        - generic [ref=e7]: Marguerite
      - button "Collapse sidebar" [expanded] [ref=e8] [cursor=pointer]:
        - img [ref=e10]
    - navigation "Control navigation" [ref=e12]:
      - button "New task" [ref=e13] [cursor=pointer]:
        - img [ref=e15]
        - generic [ref=e16]: New task
      - link "Skills & Channels" [ref=e17] [cursor=pointer]:
        - /url: /control/skills
        - img [ref=e19]
        - generic [ref=e21]: Skills & Channels
      - link "Cron" [ref=e22] [cursor=pointer]:
        - /url: /control/cron
        - img [ref=e24]
        - generic [ref=e27]: Cron
      - link "View usage" [ref=e28] [cursor=pointer]:
        - /url: /control/usage
        - img [ref=e30]
        - generic [ref=e31]: View usage
    - generic "Recent tasks" [ref=e32]:
      - generic [ref=e33]:
        - generic [ref=e34]: Recents
        - generic [ref=e35]: "1"
        - button "Search tasks (Ctrl+K)" [ref=e36] [cursor=pointer]:
          - img [ref=e38]
        - button "Manage tasks" [ref=e41] [cursor=pointer]:
          - img [ref=e43]
      - generic [ref=e49]:
        - text: Recents
        - button "Current task" [ref=e50]:
          - generic [ref=e51]: Current task
        - button "Actions for Current task" [ref=e54] [cursor=pointer]:
          - generic [ref=e55]: ⋯
    - button "Settings" [ref=e57] [cursor=pointer]:
      - img [ref=e59]
      - generic [ref=e62]: Settings
  - generic [ref=e63]:
    - banner [ref=e64]:
      - generic [ref=e66]:
        - generic:
          - 'heading "Question 1: verify the long …" [level=1]'
        - generic [ref=e68]:
          - button "Share" [ref=e69] [cursor=pointer]:
            - img [ref=e71]
          - button "Session actions" [ref=e77] [cursor=pointer]:
            - img [ref=e79]
      - generic [ref=e83]:
        - button "Manage gateway connection" [ref=e84] [cursor=pointer]: Connected
        - button "Language" [ref=e86] [cursor=pointer]:
          - img [ref=e88]
        - button "Theme" [ref=e92] [cursor=pointer]:
          - img [ref=e94]
    - main [ref=e97]:
      - generic [ref=e98]:
        - region "Chat conversation" [ref=e101]:
          - generic [ref=e102]:
            - generic [ref=e103]:
              - generic [ref=e105]: "Question 1: verify the long conversation history navigator at this exact turn."
              - generic [ref=e106]:
                - button "Copy" [ref=e107] [cursor=pointer]:
                  - img [ref=e109]
                - button "Edit" [ref=e113] [cursor=pointer]:
                  - img [ref=e115]
                - time [ref=e118]:
                  - generic [ref=e119]: Aug 11, 2026, 04:17 AM
                  - generic [ref=e120]: ·
                  - generic [ref=e121]: 24m ago
            - generic [ref=e123]:
              - generic [ref=e126]:
                - paragraph [ref=e127]: Answer 1. This is deterministic fixture paragraph 1 for turn 1; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e128]: This is deterministic fixture paragraph 2 for turn 1; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e129]: This is deterministic fixture paragraph 3 for turn 1; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e130]: This is deterministic fixture paragraph 4 for turn 1; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e131]: This is deterministic fixture paragraph 5 for turn 1; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e132]: This is deterministic fixture paragraph 6 for turn 1; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e133]: This is deterministic fixture paragraph 7 for turn 1; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e134]: This is deterministic fixture paragraph 8 for turn 1; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e136]:
                - button "Copy" [ref=e137] [cursor=pointer]:
                  - img [ref=e139]
                - button "Regenerate" [ref=e143] [cursor=pointer]:
                  - img [ref=e145]
                - time [ref=e148]:
                  - generic [ref=e149]: Aug 11, 2026, 04:18 AM
                  - generic [ref=e150]: ·
                  - generic [ref=e151]: 23m ago
            - generic [ref=e152]:
              - generic [ref=e154]: "Question 2: verify the long conversation history navigator at this exact turn."
              - generic [ref=e155]:
                - button "Copy" [ref=e156] [cursor=pointer]:
                  - img [ref=e158]
                - button "Edit" [ref=e162] [cursor=pointer]:
                  - img [ref=e164]
                - time [ref=e167]:
                  - generic [ref=e168]: Aug 11, 2026, 04:19 AM
                  - generic [ref=e169]: ·
                  - generic [ref=e170]: 22m ago
            - generic [ref=e172]:
              - generic [ref=e175]:
                - paragraph [ref=e176]: Answer 2. This is deterministic fixture paragraph 1 for turn 2; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e177]: This is deterministic fixture paragraph 2 for turn 2; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e178]: This is deterministic fixture paragraph 3 for turn 2; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e179]: This is deterministic fixture paragraph 4 for turn 2; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e180]: This is deterministic fixture paragraph 5 for turn 2; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e181]: This is deterministic fixture paragraph 6 for turn 2; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e182]: This is deterministic fixture paragraph 7 for turn 2; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e183]: This is deterministic fixture paragraph 8 for turn 2; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e185]:
                - button "Copy" [ref=e186] [cursor=pointer]:
                  - img [ref=e188]
                - button "Regenerate" [ref=e192] [cursor=pointer]:
                  - img [ref=e194]
                - time [ref=e197]:
                  - generic [ref=e198]: Aug 11, 2026, 04:20 AM
                  - generic [ref=e199]: ·
                  - generic [ref=e200]: 21m ago
            - generic [ref=e201]:
              - generic [ref=e203]: "Question 3: verify the long conversation history navigator at this exact turn."
              - generic [ref=e204]:
                - button "Copy" [ref=e205] [cursor=pointer]:
                  - img [ref=e207]
                - button "Edit" [ref=e211] [cursor=pointer]:
                  - img [ref=e213]
                - time [ref=e216]:
                  - generic [ref=e217]: Aug 11, 2026, 04:21 AM
                  - generic [ref=e218]: ·
                  - generic [ref=e219]: 20m ago
            - generic [ref=e221]:
              - generic [ref=e224]:
                - paragraph [ref=e225]: Answer 3. This is deterministic fixture paragraph 1 for turn 3; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e226]: This is deterministic fixture paragraph 2 for turn 3; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e227]: This is deterministic fixture paragraph 3 for turn 3; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e228]: This is deterministic fixture paragraph 4 for turn 3; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e229]: This is deterministic fixture paragraph 5 for turn 3; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e230]: This is deterministic fixture paragraph 6 for turn 3; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e231]: This is deterministic fixture paragraph 7 for turn 3; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e232]: This is deterministic fixture paragraph 8 for turn 3; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e234]:
                - button "Copy" [ref=e235] [cursor=pointer]:
                  - img [ref=e237]
                - button "Regenerate" [ref=e241] [cursor=pointer]:
                  - img [ref=e243]
                - time [ref=e246]:
                  - generic [ref=e247]: Aug 11, 2026, 04:22 AM
                  - generic [ref=e248]: ·
                  - generic [ref=e249]: 19m ago
            - generic [ref=e250]:
              - generic [ref=e252]: "Question 4: verify the long conversation history navigator at this exact turn."
              - generic [ref=e253]:
                - button "Copy" [ref=e254] [cursor=pointer]:
                  - img [ref=e256]
                - button "Edit" [ref=e260] [cursor=pointer]:
                  - img [ref=e262]
                - time [ref=e265]:
                  - generic [ref=e266]: Aug 11, 2026, 04:23 AM
                  - generic [ref=e267]: ·
                  - generic [ref=e268]: 18m ago
            - generic [ref=e270]:
              - generic [ref=e273]:
                - paragraph [ref=e274]: Answer 4. This is deterministic fixture paragraph 1 for turn 4; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e275]: This is deterministic fixture paragraph 2 for turn 4; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e276]: This is deterministic fixture paragraph 3 for turn 4; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e277]: This is deterministic fixture paragraph 4 for turn 4; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e278]: This is deterministic fixture paragraph 5 for turn 4; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e279]: This is deterministic fixture paragraph 6 for turn 4; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e280]: This is deterministic fixture paragraph 7 for turn 4; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e281]: This is deterministic fixture paragraph 8 for turn 4; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e283]:
                - button "Copy" [ref=e284] [cursor=pointer]:
                  - img [ref=e286]
                - button "Regenerate" [ref=e290] [cursor=pointer]:
                  - img [ref=e292]
                - time [ref=e295]:
                  - generic [ref=e296]: Aug 11, 2026, 04:24 AM
                  - generic [ref=e297]: ·
                  - generic [ref=e298]: 17m ago
            - generic [ref=e299]:
              - generic [ref=e301]: "Question 5: verify the long conversation history navigator at this exact turn."
              - generic [ref=e302]:
                - button "Copy" [ref=e303] [cursor=pointer]:
                  - img [ref=e305]
                - button "Edit" [ref=e309] [cursor=pointer]:
                  - img [ref=e311]
                - time [ref=e314]:
                  - generic [ref=e315]: Aug 11, 2026, 04:25 AM
                  - generic [ref=e316]: ·
                  - generic [ref=e317]: 16m ago
            - generic [ref=e319]:
              - generic [ref=e322]:
                - paragraph [ref=e323]: Answer 5. This is deterministic fixture paragraph 1 for turn 5; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e324]: This is deterministic fixture paragraph 2 for turn 5; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e325]: This is deterministic fixture paragraph 3 for turn 5; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e326]: This is deterministic fixture paragraph 4 for turn 5; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e327]: This is deterministic fixture paragraph 5 for turn 5; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e328]: This is deterministic fixture paragraph 6 for turn 5; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e329]: This is deterministic fixture paragraph 7 for turn 5; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e330]: This is deterministic fixture paragraph 8 for turn 5; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e332]:
                - button "Copy" [ref=e333] [cursor=pointer]:
                  - img [ref=e335]
                - button "Regenerate" [ref=e339] [cursor=pointer]:
                  - img [ref=e341]
                - time [ref=e344]:
                  - generic [ref=e345]: Aug 11, 2026, 04:26 AM
                  - generic [ref=e346]: ·
                  - generic [ref=e347]: 15m ago
            - generic [ref=e348]:
              - generic [ref=e350]: "Question 6: verify the long conversation history navigator at this exact turn."
              - generic [ref=e351]:
                - button "Copy" [ref=e352] [cursor=pointer]:
                  - img [ref=e354]
                - button "Edit" [ref=e358] [cursor=pointer]:
                  - img [ref=e360]
                - time [ref=e363]:
                  - generic [ref=e364]: Aug 11, 2026, 04:27 AM
                  - generic [ref=e365]: ·
                  - generic [ref=e366]: 14m ago
            - generic [ref=e368]:
              - generic [ref=e371]:
                - paragraph [ref=e372]: Answer 6. This is deterministic fixture paragraph 1 for turn 6; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e373]: This is deterministic fixture paragraph 2 for turn 6; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e374]: This is deterministic fixture paragraph 3 for turn 6; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e375]: This is deterministic fixture paragraph 4 for turn 6; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e376]: This is deterministic fixture paragraph 5 for turn 6; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e377]: This is deterministic fixture paragraph 6 for turn 6; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e378]: This is deterministic fixture paragraph 7 for turn 6; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e379]: This is deterministic fixture paragraph 8 for turn 6; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e381]:
                - button "Copy" [ref=e382] [cursor=pointer]:
                  - img [ref=e384]
                - button "Regenerate" [ref=e388] [cursor=pointer]:
                  - img [ref=e390]
                - time [ref=e393]:
                  - generic [ref=e394]: Aug 11, 2026, 04:28 AM
                  - generic [ref=e395]: ·
                  - generic [ref=e396]: 13m ago
            - generic [ref=e397]:
              - generic [ref=e399]: "Question 7: verify the long conversation history navigator at this exact turn."
              - generic [ref=e400]:
                - button "Copy" [ref=e401] [cursor=pointer]:
                  - img [ref=e403]
                - button "Edit" [ref=e407] [cursor=pointer]:
                  - img [ref=e409]
                - time [ref=e412]:
                  - generic [ref=e413]: Aug 11, 2026, 04:29 AM
                  - generic [ref=e414]: ·
                  - generic [ref=e415]: 12m ago
            - generic [ref=e417]:
              - generic [ref=e420]:
                - paragraph [ref=e421]: Answer 7. This is deterministic fixture paragraph 1 for turn 7; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e422]: This is deterministic fixture paragraph 2 for turn 7; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e423]: This is deterministic fixture paragraph 3 for turn 7; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e424]: This is deterministic fixture paragraph 4 for turn 7; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e425]: This is deterministic fixture paragraph 5 for turn 7; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e426]: This is deterministic fixture paragraph 6 for turn 7; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e427]: This is deterministic fixture paragraph 7 for turn 7; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e428]: This is deterministic fixture paragraph 8 for turn 7; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e430]:
                - button "Copy" [ref=e431] [cursor=pointer]:
                  - img [ref=e433]
                - button "Regenerate" [ref=e437] [cursor=pointer]:
                  - img [ref=e439]
                - time [ref=e442]:
                  - generic [ref=e443]: Aug 11, 2026, 04:30 AM
                  - generic [ref=e444]: ·
                  - generic [ref=e445]: 11m ago
            - generic [ref=e446]:
              - generic [ref=e448]: "Question 8: verify the long conversation history navigator at this exact turn."
              - generic [ref=e449]:
                - button "Copy" [ref=e450] [cursor=pointer]:
                  - img [ref=e452]
                - button "Edit" [ref=e456] [cursor=pointer]:
                  - img [ref=e458]
                - time [ref=e461]:
                  - generic [ref=e462]: Aug 11, 2026, 04:31 AM
                  - generic [ref=e463]: ·
                  - generic [ref=e464]: 10m ago
            - generic [ref=e466]:
              - generic [ref=e469]:
                - paragraph [ref=e470]: Answer 8. This is deterministic fixture paragraph 1 for turn 8; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e471]: This is deterministic fixture paragraph 2 for turn 8; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e472]: This is deterministic fixture paragraph 3 for turn 8; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e473]: This is deterministic fixture paragraph 4 for turn 8; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e474]: This is deterministic fixture paragraph 5 for turn 8; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e475]: This is deterministic fixture paragraph 6 for turn 8; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e476]: This is deterministic fixture paragraph 7 for turn 8; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e477]: This is deterministic fixture paragraph 8 for turn 8; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e479]:
                - button "Copy" [ref=e480] [cursor=pointer]:
                  - img [ref=e482]
                - button "Regenerate" [ref=e486] [cursor=pointer]:
                  - img [ref=e488]
                - time [ref=e491]:
                  - generic [ref=e492]: Aug 11, 2026, 04:32 AM
                  - generic [ref=e493]: ·
                  - generic [ref=e494]: 9m ago
            - generic [ref=e495]:
              - generic [ref=e497]: "Question 9: verify the long conversation history navigator at this exact turn."
              - generic [ref=e498]:
                - button "Copy" [ref=e499] [cursor=pointer]:
                  - img [ref=e501]
                - button "Edit" [ref=e505] [cursor=pointer]:
                  - img [ref=e507]
                - time [ref=e510]:
                  - generic [ref=e511]: Aug 11, 2026, 04:33 AM
                  - generic [ref=e512]: ·
                  - generic [ref=e513]: 8m ago
            - generic [ref=e515]:
              - generic [ref=e518]:
                - paragraph [ref=e519]: Answer 9. This is deterministic fixture paragraph 1 for turn 9; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e520]: This is deterministic fixture paragraph 2 for turn 9; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e521]: This is deterministic fixture paragraph 3 for turn 9; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e522]: This is deterministic fixture paragraph 4 for turn 9; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e523]: This is deterministic fixture paragraph 5 for turn 9; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e524]: This is deterministic fixture paragraph 6 for turn 9; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e525]: This is deterministic fixture paragraph 7 for turn 9; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e526]: This is deterministic fixture paragraph 8 for turn 9; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e528]:
                - button "Copy" [ref=e529] [cursor=pointer]:
                  - img [ref=e531]
                - button "Regenerate" [ref=e535] [cursor=pointer]:
                  - img [ref=e537]
                - time [ref=e540]:
                  - generic [ref=e541]: Aug 11, 2026, 04:34 AM
                  - generic [ref=e542]: ·
                  - generic [ref=e543]: 7m ago
            - generic [ref=e544]:
              - generic [ref=e546]: "Question 10: verify the long conversation history navigator at this exact turn."
              - generic [ref=e547]:
                - button "Copy" [ref=e548] [cursor=pointer]:
                  - img [ref=e550]
                - button "Edit" [ref=e554] [cursor=pointer]:
                  - img [ref=e556]
                - time [ref=e559]:
                  - generic [ref=e560]: Aug 11, 2026, 04:35 AM
                  - generic [ref=e561]: ·
                  - generic [ref=e562]: 6m ago
            - generic [ref=e564]:
              - generic [ref=e567]:
                - paragraph [ref=e568]: Answer 10. This is deterministic fixture paragraph 1 for turn 10; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e569]: This is deterministic fixture paragraph 2 for turn 10; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e570]: This is deterministic fixture paragraph 3 for turn 10; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e571]: This is deterministic fixture paragraph 4 for turn 10; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e572]: This is deterministic fixture paragraph 5 for turn 10; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e573]: This is deterministic fixture paragraph 6 for turn 10; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e574]: This is deterministic fixture paragraph 7 for turn 10; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e575]: This is deterministic fixture paragraph 8 for turn 10; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e577]:
                - button "Copy" [ref=e578] [cursor=pointer]:
                  - img [ref=e580]
                - button "Regenerate" [ref=e584] [cursor=pointer]:
                  - img [ref=e586]
                - time [ref=e589]:
                  - generic [ref=e590]: Aug 11, 2026, 04:36 AM
                  - generic [ref=e591]: ·
                  - generic [ref=e592]: 5m ago
            - generic [ref=e593]:
              - generic [ref=e595]: "Question 11: verify the long conversation history navigator at this exact turn."
              - generic [ref=e596]:
                - button "Copy" [ref=e597] [cursor=pointer]:
                  - img [ref=e599]
                - button "Edit" [ref=e603] [cursor=pointer]:
                  - img [ref=e605]
                - time [ref=e608]:
                  - generic [ref=e609]: Aug 11, 2026, 04:37 AM
                  - generic [ref=e610]: ·
                  - generic [ref=e611]: 4m ago
            - generic [ref=e613]:
              - generic [ref=e616]:
                - paragraph [ref=e617]: Answer 11. This is deterministic fixture paragraph 1 for turn 11; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e618]: This is deterministic fixture paragraph 2 for turn 11; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e619]: This is deterministic fixture paragraph 3 for turn 11; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e620]: This is deterministic fixture paragraph 4 for turn 11; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e621]: This is deterministic fixture paragraph 5 for turn 11; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e622]: This is deterministic fixture paragraph 6 for turn 11; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e623]: This is deterministic fixture paragraph 7 for turn 11; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e624]: This is deterministic fixture paragraph 8 for turn 11; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e626]:
                - button "Copy" [ref=e627] [cursor=pointer]:
                  - img [ref=e629]
                - button "Regenerate" [ref=e633] [cursor=pointer]:
                  - img [ref=e635]
                - time [ref=e638]:
                  - generic [ref=e639]: Aug 11, 2026, 04:38 AM
                  - generic [ref=e640]: ·
                  - generic [ref=e641]: 3m ago
            - generic [ref=e642]:
              - generic [ref=e644]: "Question 12: verify the long conversation history navigator at this exact turn."
              - generic [ref=e645]:
                - button "Copy" [ref=e646] [cursor=pointer]:
                  - img [ref=e648]
                - button "Edit" [ref=e652] [cursor=pointer]:
                  - img [ref=e654]
                - time [ref=e657]:
                  - generic [ref=e658]: Aug 11, 2026, 04:39 AM
                  - generic [ref=e659]: ·
                  - generic [ref=e660]: 2m ago
            - generic [ref=e662]:
              - generic [ref=e665]:
                - paragraph [ref=e666]: Answer 12. This is deterministic fixture paragraph 1 for turn 12; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e667]: This is deterministic fixture paragraph 2 for turn 12; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e668]: This is deterministic fixture paragraph 3 for turn 12; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e669]: This is deterministic fixture paragraph 4 for turn 12; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e670]: This is deterministic fixture paragraph 5 for turn 12; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e671]: This is deterministic fixture paragraph 6 for turn 12; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e672]: This is deterministic fixture paragraph 7 for turn 12; it makes the thread tall enough to exercise scroll and resize anchoring.
                - paragraph [ref=e673]: This is deterministic fixture paragraph 8 for turn 12; it makes the thread tall enough to exercise scroll and resize anchoring.
              - generic [ref=e675]:
                - button "Copy" [ref=e676] [cursor=pointer]:
                  - img [ref=e678]
                - button "Regenerate" [ref=e682] [cursor=pointer]:
                  - img [ref=e684]
                - button "Fork from here" [ref=e687] [cursor=pointer]:
                  - img [ref=e689]
                - time [ref=e694]:
                  - generic [ref=e695]: Aug 11, 2026, 04:40 AM
                  - generic [ref=e696]: ·
                  - generic [ref=e697]: 1m ago
        - generic [ref=e700]:
          - generic [ref=e701]:
            - textbox "Message to send" [active] [ref=e703]:
              - /placeholder: Send a message...
            - generic [ref=e705]:
              - generic [ref=e706]:
                - button "Add" [ref=e708] [cursor=pointer]:
                  - img [ref=e710]
                - button "Model routing" [ref=e712] [cursor=pointer]:
                  - img [ref=e714]
                  - generic: NEW
                - button "Execution mode" [ref=e721] [cursor=pointer]:
                  - img [ref=e723]
                - button "More" [ref=e726] [cursor=pointer]:
                  - img [ref=e728]
              - button "Send" [ref=e733] [cursor=pointer]:
                - img [ref=e735]
          - note [ref=e738]: Content generated by AI may be inaccurate. For reference only; verify important information.
        - status [ref=e739]
  - status [ref=e740]
```

# Test source

```ts
  1   | import { expect, test, type Page } from '@playwright/test'
  2   | 
  3   | const CONTROL_URL = '/control/'
  4   | const SESSION_KEY = 'agent:main:webchat:e2e-long-history-12-turns'
  5   | 
  6   | function longHistoryMessages() {
  7   |   const now = Math.floor(Date.now() / 1000)
  8   |   return Array.from({ length: 12 }, (_, index) => {
  9   |     const turn = index + 1
  10  |     return [
  11  |       {
  12  |         role: 'user',
  13  |         text: `Question ${turn}: verify the long conversation history navigator at this exact turn.`,
  14  |         id: `history-user-${turn}`,
  15  |         timestamp: now - (24 - index * 2) * 60,
  16  |       },
  17  |       {
  18  |         role: 'assistant',
  19  |         text: `Answer ${turn}. ${Array.from({ length: 8 }, (__, paragraph) => (
  20  |           `This is deterministic fixture paragraph ${paragraph + 1} for turn ${turn}; it makes the thread tall enough to exercise scroll and resize anchoring.`
  21  |         )).join('\n\n')}`,
  22  |         id: `history-assistant-${turn}`,
  23  |         timestamp: now - (23 - index * 2) * 60,
  24  |       },
  25  |     ]
  26  |   }).flat()
  27  | }
  28  | 
  29  | async function seedLongHistory(page: Page) {
  30  |   await page.route('**/api/approvals', route => route.fulfill({
  31  |     status: 200,
  32  |     contentType: 'application/json',
  33  |     body: JSON.stringify({ pending: [] }),
  34  |   }))
  35  | 
  36  |   await page.routeWebSocket(/\/ws$/, ws => {
  37  |     ws.send(JSON.stringify({ type: 'event', event: 'connect.challenge', payload: {} }))
  38  |     ws.onMessage(message => {
  39  |       try {
  40  |         const frame = JSON.parse(String(message))
  41  |         if (frame?.type !== 'req') return
  42  |         if (frame.method === 'connect') {
  43  |           ws.send(JSON.stringify({
  44  |             protocol: 3,
  45  |             policy: { tick_interval_ms: 30000 },
  46  |           }))
  47  |           return
  48  |         }
  49  | 
  50  |         const payloads: Record<string, unknown> = {
  51  |           'agents.list': { agents: [] },
  52  |           'chat.history': { messages: longHistoryMessages(), has_more: false },
  53  |           'commands.list_for_surface': { commands: [] },
  54  |           'config.get': {
  55  |             squilla_router: { enabled: false, rollout_phase: 'observe', tiers: {} },
  56  |             permissions: {},
  57  |             skills: {},
  58  |           },
  59  |           'sessions.list': { sessions: [], has_more: false },
  60  |           'sessions.messages.subscribe': {
  61  |             subscribed: true,
  62  |             replay_complete: true,
  63  |             current_stream_seq: 0,
  64  |             run_status: 'idle',
  65  |           },
  66  |           'usage.status': { sessions: [] },
  67  |         }
  68  | 
  69  |         ws.send(JSON.stringify({
  70  |           type: 'res',
  71  |           id: frame.id,
  72  |           ok: true,
  73  |           payload: payloads[String(frame.method)] ?? {},
  74  |         }))
  75  |       } catch {}
  76  |     })
  77  |   })
  78  | }
  79  | 
  80  | test.describe('Long conversation history rail', () => {
  81  |   test('769px compact dock preserves a readable chat title without horizontal overflow', async ({ page }) => {
  82  |     await page.addInitScript(() => localStorage.removeItem('marguerite.sidebar.width.v1'))
  83  |     await page.setViewportSize({ width: 769, height: 900 })
  84  |     await seedLongHistory(page)
  85  |     await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(SESSION_KEY))
  86  |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  87  | 
  88  |     const main = page.locator('#app-main')
  89  |     const sidebar = page.locator('.sidebar')
  90  |     const header = page.locator('.chat-header')
  91  |     const title = page.locator('.chat-header .chat-label')
  92  | 
  93  |     await expect(main).toHaveClass(/main--sidebar-compact/)
  94  |     await expect(page.getByTestId('sidebar-resizer')).toHaveCount(0)
  95  |     await expect.poll(async () => Math.round((await sidebar.boundingBox())?.width || 0)).toBe(260)
> 96  |     await expect.poll(async () => Math.round((await title.boundingBox())?.width || 0)).toBeGreaterThanOrEqual(96)
      |                                                                                        ^ Error: expect(received).toBeGreaterThanOrEqual(expected)
  97  |     await expect.poll(() => header.evaluate(element => (
  98  |       element.scrollWidth <= element.clientWidth
  99  |     ))).toBe(true)
  100 |     await expect(page.locator('.chat-header .chat-share-btn__label').first()).toBeHidden()
  101 |     await expect(page.locator('.topbar .lang-menu-current')).toBeHidden()
  102 |   })
  103 | 
  104 |   test('reduced motion keeps a static local arrival guide without painting the whole row', async ({ page }) => {
  105 |     await page.emulateMedia({ reducedMotion: 'reduce' })
  106 |     await page.addInitScript(() => localStorage.removeItem('marguerite.sidebar.width.v1'))
  107 |     await page.setViewportSize({ width: 1440, height: 900 })
  108 |     await seedLongHistory(page)
  109 |     await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(SESSION_KEY))
  110 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  111 | 
  112 |     const markers = page.getByTestId('conversation-minimap-marker')
  113 |     await expect(markers).toHaveCount(12, { timeout: 10000 })
  114 |     await markers.nth(0).click()
  115 | 
  116 |     const firstTurn = page.locator('[data-chat-turn-key="history-user-1"]')
  117 |     await expect(firstTurn).toHaveClass(/is-history-target/, { timeout: 3000 })
  118 |     const arrivalPaint = await firstTurn.evaluate((element) => {
  119 |       const stack = element.querySelector('.msg-user-stack')
  120 |       const rowStyle = getComputedStyle(element)
  121 |       const guideStyle = stack ? getComputedStyle(stack, '::after') : null
  122 |       const maxDuration = (value: string) => Math.max(
  123 |         ...value.split(',').map(part => Number.parseFloat(part) || 0),
  124 |       )
  125 |       return {
  126 |         rowBackground: rowStyle.backgroundColor,
  127 |         rowShadow: rowStyle.boxShadow,
  128 |         guideAnimation: guideStyle?.animationName || '',
  129 |         guideOpacity: guideStyle?.opacity || '',
  130 |         guideWidth: guideStyle?.width || '',
  131 |         maxAnimationDuration: maxDuration(guideStyle?.animationDuration || '0s'),
  132 |         maxTransitionDuration: maxDuration(guideStyle?.transitionDuration || '0s'),
  133 |       }
  134 |     })
  135 |     expect(arrivalPaint).toMatchObject({
  136 |       rowBackground: 'rgba(0, 0, 0, 0)',
  137 |       rowShadow: 'none',
  138 |       guideAnimation: 'none',
  139 |       guideOpacity: '0.78',
  140 |       guideWidth: '2px',
  141 |     })
  142 |     expect(arrivalPaint.maxAnimationDuration).toBeLessThanOrEqual(0.00001)
  143 |     expect(arrivalPaint.maxTransitionDuration).toBeLessThanOrEqual(0.00001)
  144 |   })
  145 | 
  146 |   test('keeps idle markers compact until pointer hover or keyboard focus', async ({ page }) => {
  147 |     await page.addInitScript(() => localStorage.removeItem('marguerite.sidebar.width.v1'))
  148 |     await page.setViewportSize({ width: 1440, height: 900 })
  149 |     await seedLongHistory(page)
  150 |     await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(SESSION_KEY))
  151 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  152 | 
  153 |     const thread = page.locator('.chat-thread')
  154 |     const markers = page.getByTestId('conversation-minimap-marker')
  155 |     await expect(markers).toHaveCount(12, { timeout: 10000 })
  156 | 
  157 |     await thread.evaluate((element) => {
  158 |       element.scrollTop = 0
  159 |       element.dispatchEvent(new Event('scroll'))
  160 |     })
  161 | 
  162 |     const markerStyles = () => markers.evaluateAll(elements => elements.map(element => ({
  163 |       active: element.getAttribute('aria-current') === 'location',
  164 |       opacity: element.style.getPropertyValue('--conversation-minimap-line-opacity'),
  165 |       scaleX: element.style.getPropertyValue('--conversation-minimap-line-scale-x'),
  166 |       scaleY: element.style.getPropertyValue('--conversation-minimap-line-scale-y'),
  167 |     })))
  168 | 
  169 |     await expect.poll(markerStyles).toEqual([
  170 |       { active: true, opacity: '1.0000', scaleX: '0.2667', scaleY: '1.0000' },
  171 |       ...Array.from({ length: 11 }, () => ({
  172 |         active: false,
  173 |         opacity: '0.4500',
  174 |         scaleX: '0.2667',
  175 |         scaleY: '0.5000',
  176 |       })),
  177 |     ])
  178 | 
  179 |     await thread.evaluate((element) => {
  180 |       element.scrollTop = 1000
  181 |       element.dispatchEvent(new Event('scroll'))
  182 |     })
  183 |     await expect.poll(async () => (
  184 |       await markerStyles()
  185 |     ).findIndex(marker => marker.active)).toBeGreaterThan(0)
  186 |     expect((await markerStyles()).every(marker => marker.scaleX === '0.2667')).toBe(true)
  187 |     const activeAfterScroll = (await markerStyles()).findIndex(marker => marker.active)
  188 | 
  189 |     await markers.nth(5).hover()
  190 |     await expect.poll(async () => (await markerStyles())[5]?.scaleX).toBe('1.0000')
  191 |     const hoveredStyles = await markerStyles()
  192 |     expect(Number(hoveredStyles[4]?.scaleX)).toBeGreaterThan(0.2667)
  193 |     expect(hoveredStyles[activeAfterScroll]).toMatchObject({
  194 |       active: true,
  195 |       opacity: '1.0000',
  196 |       scaleY: '1.0000',
```