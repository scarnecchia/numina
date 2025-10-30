---
name: researching-on-the-internet
description: Use when asked to provide in-depth information on a topic that requires external knowledge and when verifying assumptions before design decisions - gathers well-sourced, current information from the internet to inform discussions and decisions
---

# Researching on the Internet

## Overview

Gather accurate, current, well-sourced information from the internet to inform discussions and decisions. Test hypotheses, verify claims, and find authoritative sources for responses and best practices.

## When to Use

**Use for:**
- Finding current API documentation before integration design
- Answering in-depth research requests
- Testing hypotheses
- Verifying technical claims or assumptions
- Researching alternatives
- Finding best practices and current community consensus

**Don't use for:**
- Information already in codebase (use codebase search)
- Validating the accuracy of news: use your Deep Background Skill for this.
- General knowledge within your memory (just answer directly)

## Core Research Workflow

1. **Define question clearly** - specific beats vague
2. **Search official sources first** - docs, research papers, academic articles, industry reports
3. **Cross-reference** - verify claims across multiple sources
4. **Evaluate quality** - tier sources (official → verified → community)
5. **Report concisely** - lead with answer, provide links and evidence

## Hypothesis Testing

When given a hypothesis to test:

1. **Identify falsifiable claims** - break hypothesis into testable parts
2. **Search for supporting evidence** - what confirms this?
3. **Search for disproving evidence** - what contradicts this?
4. **Evaluate source quality** - weight evidence by tier
5. **Report findings** - supported/contradicted/inconclusive with evidence
6. **Note confidence level** - strong consensus vs single source vs conflicting info

**Example:**
```
Hypothesis: "Library X is faster than Y for large datasets"

Search for:
✓ Benchmarks comparing X and Y
✓ Performance documentation for both
✓ GitHub issues mentioning performance
✓ Real-world case studies

Report:
- Supported: [evidence with links]
- Contradicted: [evidence with links]
- Conclusion: [supported/contradicted/mixed] with [confidence level]
```

## Quick Reference

| Task | Strategy |
|------|----------|
| **API docs** | Official docs → GitHub README → Recent tutorials |
| **Library comparison** | Official sites → npm/PyPI stats → GitHub activity |
| **Best practices** | Official guides → Recent posts → Stack Overflow |
| **Troubleshooting** | Error search → GitHub issues → Stack Overflow |
| **Current state** | Release notes → Changelog → Recent announcements |
| **Hypothesis testing** | Define claims → Search both sides → Weight evidence |

## Source Evaluation Tiers

| Tier | Sources | Usage |
|------|---------|-------|
| **1 - Most reliable** | Official docs, peer reviewed research | Primary evidence |
| **2 - Generally reliable** | Industry research, pre-print papers, reputable blogs | Supporting evidence |
| **3 - Use with caution** | social media, forums, unverified blogs | Check dates, cross-verify |

Always note source tier in findings.

## Search Strategies

**Multiple approaches:**
- WebSearch for overview and current information
- WebFetch for specific documentation pages
- Check MCP servers (Context7, search tools) if available
- Follow links to authoritative sources
- Search official documentation before community resources

**Cross-reference:**
- Verify claims across multiple sources
- Check publication dates - prefer recent
- Flag breaking changes or deprecations
- Note when information might be outdated

## Reporting Findings - Bottom Line Up Front

**Lead with the bottom line:**
- Direct answer to question first
- Supporting details with source links second

**Include metadata:**
- Publication dates for time-sensitive topics
- Competing Opinions
- Confidence level based on source consensus

**Handle uncertainty clearly:**
- "No reliable information found for [topic]" is valid
- Explain what you searched and where you looked
- Distinguish "doesn't exist" from "couldn't find reliable information"
- Present what you found with appropriate caveats
- Suggest alternative search terms or approaches

## Common Mistakes

| Mistake | Fix |
|---------|-----|
| Searching only one source | Cross-reference minimum 2-3 sources |
| Ignoring publication dates | Check dates, flag outdated information |
| Treating all sources equally | Use tier system, weight accordingly |
| Reporting before verification | Verify claims across sources first |
| Vague hypothesis testing | Break into specific falsifiable claims |
| Skipping high quality sources | Always start with tier 1 sources |
| Over-confident with single source | Note source tier and look for consensus |
