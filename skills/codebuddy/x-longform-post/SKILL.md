---
name: x-longform-post
description: Write long-form X (Twitter) posts and threads in a founder/CEO voice. Use when drafting X articles, long tweets, thought leadership threads, or viral content. Produces contrarian, data-backed posts with ASCII diagrams and code block visuals. Includes mandatory AI humanizer pass (24-pattern detector) before finalizing.
description_zh: "撰写 X(Twitter) 长文，创始人语气 + AI 去味检测"
description_en: "Write long-form X posts in founder voice with AI humanizer detection"
version: 1.0.0
homepage: https://github.com/ericosiu/ai-marketing-skills
---

# X Long-Form Post Writer

Write posts for X in your founder/CEO's authentic voice. Every post should feel like a real person wrote it — not a content team, not a bot.

See `references/founder-voice.md` for the founder voice template. Customize it with your founder's real patterns.

---

## Voice Rules

- Simple declarative sentences. Short paragraphs.
- Contrarian angles backed by specific numbers and real examples.
- No corporate speak. No "I'm excited to share." No emoji in body text.
- Open with a hook that stops the scroll — contrarian claim, surprising number, or uncomfortable truth.
- End with a payoff: uncomfortable truth → "worth it" resolution.

---

## Structure

1. **Hook** (1-2 lines) — Contrarian claim or surprising stat
2. **Setup** (2-3 lines) — Establish credibility/context fast
3. **Sections** — Each follows: problem → what actually happened → fix/lesson
4. **ASCII diagram** — At least one per post (see below)
5. **Uncomfortable truth** — The insight most people avoid
6. **Payoff** — Was it worth it? Yes, and here's why.

---

## ASCII Diagrams (MANDATORY)

Every post MUST include at least one ASCII diagram in a code block. These break up walls of text and make complex systems visual.

Use box-drawing characters:
```
┌─────────┐    ┌─────────┐    ┌─────────┐
│  Input  │───►│ Process │───►│ Output  │
└─────────┘    └─────────┘    └─────────┘
```

Diagram types to use:
- **System architecture** — boxes connected by arrows showing how components relate
- **Before/after** — side-by-side comparison of old vs new state
- **Flow diagrams** — decision trees, pipelines, sequences
- **Hierarchy** — org charts, priority stacks, dependency trees
- **Metrics** — simple bar charts using block characters (█ ▓ ░)

Keep diagrams:
- Under 40 chars wide (mobile rendering)
- Simple enough to parse in 3 seconds
- Labeled clearly — no ambiguous boxes

Example — system flow:
```
Input (60s)
    │
    ▼
┌──────────┐
│ Process  │ step 1
└────┬─────┘
     ▼
┌──────────┐
│ Dispatch │ step 2
└────┬─────┘
     ▼
  Output
```

Example — metrics visualization:
```
Performance by Category:
Category A   ████████████ 100%
Category B   ████████░░░░  67%
Category C   ░░░░░░░░░░░░   0%
```

---

## Formatting for X

- X articles support markdown-like formatting in long posts
- Use code blocks (```) for ASCII art — they render in monospace on X
- Bold with asterisks where supported
- Keep paragraphs to 1-3 sentences max
- Line breaks between every thought

---

## Content Sources

Pull from real data whenever possible:
- Real metrics from your business
- Specific incidents and debugging stories
- Actual decisions made and why

Never fabricate metrics. Use real numbers or don't use numbers.

---

## Input Format

User provides:
- **Topic**: What the post is about
- **Angle**: The contrarian or unique framing
- **Source material**: Real examples, data, incidents (optional)

---

## Output

Deliver the complete post ready to paste into X. No preamble, no "here's your post" — just the post itself.

If the post would work better as a thread (>1500 chars), split into numbered tweets with each one standalone valuable.

---

## Reference

See `references/founder-voice.md` for extended voice examples and patterns. Customize with your founder's real voice.

---

## Humanizer Checklist (MANDATORY — Run Before Finalizing)

Before returning any X article draft, check against ALL 24 humanizer patterns. If any pattern is detected, rewrite that section.

For the full humanizer expert scoring rubric, see: `../content-ops/experts/humanizer.md`

### CRITICAL: No "Not X, It's Y" Constructions
Never write "This is not X. This is Y." or "That is not X, that is Y." or any variant. These are the #1 AI slop tell. Say what something IS directly. Don't define by negation.

### Banned Vocabulary (never use these)
delve, tapestry, landscape (abstract), leverage, multifaceted, nuanced, pivotal, realm, robust, seamless, testament, transformative, underscore (verb), utilize, whilst, keen, embark, comprehensive, intricate, commendable, meticulous, paramount, groundbreaking, innovative, cutting-edge, synergy, holistic, paradigm, ecosystem, Additionally, crucial, enduring, enhance, fostering, garner, highlight (verb), interplay, intricacies, showcase, vibrant, valuable, profound, renowned, breathtaking, nestled, stunning

### Pattern Checklist
1. ☐ No significance inflation ("pivotal moment", "stands as", "is a testament")
2. ☐ No undue notability claims (listing media mentions without context)
3. ☐ No superficial -ing phrases ("highlighting", "showcasing", "underscoring")
4. ☐ No promotional language ("boasts", "vibrant", "profound", "commitment to")
5. ☐ No vague attributions ("Experts believe", "Industry reports suggest")
6. ☐ No formulaic "despite challenges... continues to" structures
7. ☐ No AI vocabulary clustering (multiple banned words in one paragraph)
8. ☐ No copula avoidance ("serves as", "stands as" — just use "is")
9. ☐ No negative parallelisms ("It's not just X, it's Y")
10. ☐ No rule-of-three forcing (triple adjectives, triple parallel clauses)
11. ☐ No synonym cycling (varying terms for the same thing unnecessarily)
12. ☐ No false ranges ("from X to Y" on no meaningful scale)
13. ☐ No em dash overuse (max 1 per 200 words)
14. ☐ No mechanical boldface emphasis
15. ☐ No inline-header vertical lists (bolded label + colon pattern)
16. ☐ No Title Case In Every Heading
17. ☐ No emoji decoration on headings/bullets
18. ☐ No curly quotation marks
19. ☐ No collaborative artifacts ("I hope this helps", "Let me know")
20. ☐ No knowledge-cutoff disclaimers
21. ☐ No sycophantic tone ("Great question!")
22. ☐ No filler phrases ("In order to", "It is important to note")
23. ☐ No excessive hedging ("could potentially", "might have some effect")
24. ☐ No generic positive conclusions ("The future looks bright", "Exciting times ahead")

### Humanizer Scoring

Start at 100. Deduct points per the rubric in `../content-ops/experts/humanizer.md`.

- **90-100**: Human-sounding. Clean. Ship it.
- **70-89**: Minor AI tells. Quick fixes needed.
- **50-69**: Obvious AI patterns. Significant rewrite needed.
- **0-49**: Full rewrite.
