# MACO: Multi-Agent Context Orchestration

**Novel research direction: token optimization as an orchestrator-level resource-allocation
problem, not a per-agent compression problem.**

Status: research design + working prototype in this repository
(`crates/tokenopt-core/src/orchestrator.rs`, `orchestrator_sim.rs`,
`POST /v1/orchestrator/compile`, `ctxc bench orchestrator`).

---

## 1. Motivation

Every existing context-reduction technique we know of — summarization (MemGPT,
MEM1), prompt compression (LLMLingua-2), retention policies (ACON), masking and
folding (SGCC, context folding), KV-cache reuse — operates on **one agent's
transcript at a time**. TokenOpt's existing pipeline is state of the art in that
setting: parse one transcript, transform, verify sufficiency, emit.

Multi-agent systems (supervisor/worker trees in LangGraph, CrewAI crews,
AutoGen group chats, swarm orchestrators) break the assumption that contexts
are independent. Measured across the whole orchestrator, token waste has
**cross-agent structure** that no per-agent compiler can see:

1. **Duplicated observations.** Workers exploring the same repository read the
   same files, fetch the same docs, and re-run the same commands. Each copy is
   compressed *independently*, but N agents still each carry one copy of the
   same payload through every subsequent model call.
2. **Mispriced budgets.** Orchestrators give every agent the same context
   window slice regardless of marginal value. A supervisor tracking the whole
   task tree and an idle worker get identical budgets; tokens trimmed from the
   supervisor are worth far more than tokens trimmed from the idle worker.
3. **Sender-biased handoffs.** Handoff summaries are written from the
   *producer's* perspective: everything the sub-agent did. The *consumer*
   usually needs a small projection of that record. Nobody compresses the
   communication **edges** of the agent graph against the receiver's needs.
4. **No feedback loop.** When a compiler cuts too much for one agent, that
   agent silently degrades (re-reads files, fails sufficiency). These signals
   are observable at the orchestrator but are not fed back into per-agent
   compression aggressiveness anywhere in the literature or in practice.

**Thesis.** Treat the orchestrator's total context spend as a single economy:
a global token budget allocated across agents by marginal utility, a shared
content-addressed store that makes redundant observations free, handoffs
compressed against consumer contracts, and a regret feedback loop that keeps
allocations honest. We call this **MACO — Multi-Agent Context Orchestration**.

## 2. Problem statement

An orchestrator runs agents \(i = 1..n\), each with transcript \(M_i\) and a
compression operator \(C(M_i, b_i)\) that emits a context fitting budget
\(b_i\). Given a global budget \(B\) per orchestrator round, choose budgets and
shared-content policy to

\[
\max \sum_i U_i(C(M_i, b_i)) \quad \text{s.t.} \quad \sum_i b_i \le B,
\]

where \(U_i\) is agent \(i\)'s task utility. \(U_i\) is unobservable at compile
time, so MACO uses a surrogate with diminishing returns,
\(U_i(b) \approx w_i \ln(1 + b)\), where the weight \(w_i\) composes three
observable factors (§3.1). Cross-agent dedup changes the constraint itself: a
payload shared by \(k\) agents costs one canonical copy plus \(k-1\) constant-size
references instead of \(k\) full copies.

## 3. The four pillars

### 3.1 Global budget allocation by water-filling (GBA)

`allocate_budgets` / `AllocationStrategy::WaterFilling` in `orchestrator.rs`.

Weighted log-utility maximization under a sum constraint has the closed-form
KKT solution \(b_i = \mathrm{clamp}(w_i/\lambda - 1,\ \mathrm{floor}_i,\ \mathrm{demand}_i)\);
we solve for the water level \(\lambda\) by bisection. Properties the prototype
enforces and tests:

- never allocates more than the global budget;
- agents whose demand is below their fair share return surplus, which flows to
  high-demand agents (uniform splits burn this surplus);
- per-agent floors prevent starvation (sufficiency beats budget);
- weights compose `priority x role multiplier x regret boost`:
  supervisors 1.5x, critics 1.2x, workers 1.0x, memory agents 0.8x.

*Novelty:* budget allocation across **concurrently active agent contexts** as a
utility-maximization problem. Prior art allocates within one context (which
blocks to keep) — not across agents.

### 3.2 Cross-agent content-addressed dedup (CAD)

`dedup_across_agents` in `orchestrator.rs`.

Tool results above a size threshold are content-hashed (FNV-1a 128).
The first sighting across the whole agent set becomes the **canonical copy**
and stays inline; every other sighting in any agent is replaced by a 160-char
preview plus a shared reference `ref://shared/{hash}` whose payload lives once
in the orchestrator's cold store. Two safety rules:

- each agent's newest `dedup_keep_recent` (default 1) tool results are never
  masked — the active working set stays live;
- masked copies are always recoverable through the existing rehydration API
  (`/v1/rehydrate`), so dedup is lossless at the system level.

*Novelty:* dedup across **separate model contexts** with first-writer-wins
canonicalization and orchestrator-level rehydration. Prompt-caching dedups
identical *prefixes* within one model; nothing dedups *observations* across
cooperating agents' contexts.

### 3.3 Consumer-contract handoff compression (CCH)

`compress_handoff_for_consumer` in `orchestrator.rs`.

A handoff (`FoldRecord`) is an edge in the agent communication graph. MACO
compresses the edge against the **receiver's** declared required slots (the
same slot vocabulary the sufficiency oracle already uses): status, subgoal, and
budget accounting always survive; artifacts, decisions, preconditions, and open
issues survive only if they match a consumer slot. No declared contract means
pass-through — the optimization is strictly opt-in per edge.

*Novelty:* sufficiency-as-interface. Existing handoff schemes (fold records,
A2A-style messages) compress from the sender's view; MACO is, to our
knowledge, the first to budget the *edges* of the topology by the consumer's
slot contract.

### 3.4 Adaptive regret controller (ARC)

`AgentFeedback` + `effective_weight` in `orchestrator.rs`.

Two cheap, already-observable signals act as implicit regret that compression
was too aggressive for an agent: sufficiency-oracle failures and rehydration
requests (an agent asking for masked content back is direct evidence it needed
that content). Each signal raises the agent's allocation weight by 25% (capped
at 3x) for subsequent rounds, so the water-filling ledger automatically shifts
budget toward agents the compiler has been hurting.

*Novelty:* closing the loop between *recovery traffic* and *compression
aggressiveness*, per agent, at the orchestrator. Static pipelines (including
single-agent TokenOpt) have no such feedback channel.

## 4. Prototype

| Component | Location |
|-----------|----------|
| Agent model, ledger, dedup, handoffs, feedback | `crates/tokenopt-core/src/orchestrator.rs` |
| `compile_multi_agent` (dedup -> allocate -> per-agent compile) | same |
| Supervisor + workers simulation, 3-strategy comparison | `crates/tokenopt-core/src/orchestrator_sim.rs` |
| HTTP API | `POST /v1/orchestrator/compile` (`tokenopt-server`) |
| CLI benchmark | `ctxc bench orchestrator` |
| Tests | `crates/tokenopt-core/tests/orchestrator.rs` |

`compile_multi_agent` runs per-agent compilation with `soft_sufficiency` so a
single failing agent degrades to passthrough instead of aborting the round, and
reuses the entire existing single-agent pipeline (masking, summarization,
folding, budget trim) under each agent's allocated budget.

### HTTP example

```bash
curl -s localhost:8787/v1/orchestrator/compile -H 'content-type: application/json' -d '{
  "agents": [
    {"agent_id": "supervisor", "role": "supervisor", "messages": [...]},
    {"agent_id": "worker-1", "role": "worker", "priority": 1.0,
     "feedback": {"rehydration_requests": 2}, "messages": [...]}
  ],
  "options": {"global_token_budget": 64000, "allocation": "water_filling"}
}'
```

The response carries per-agent compiled messages, the budget ledger, the dedup
report (including shared refs), and aggregate savings.

## 5. Evaluation

Benchmark: 1 supervisor + 8 workers, 12 rounds, 6 KB tool payloads, sweeping
the fraction of worker reads that hit shared artifacts. Three strategies per
round: **baseline** (no compilation), **independent** (state of the art:
per-agent compile, uniform budget split, no cross-agent awareness), **MACO**
(cross-agent dedup + water-filling). Cumulative tokens across rounds
(`ctxc bench orchestrator --workers 8 --rounds 12 --global-budget 96000`):

| Shared reads | Baseline | Independent | MACO | MACO vs baseline (final) | MACO vs independent (final) |
|---:|---:|---:|---:|---:|---:|
| 25% | 964,567 | 309,523 | 293,252 | 82.4% | 5.2% |
| 50% | 964,021 | 309,198 | 261,325 | 84.3% | 15.6% |
| 75% | 963,475 | 308,864 | 229,389 | 86.3% | 26.0% |
| 100% | 962,929 | 308,539 | 197,445 | 88.2% | 36.4% |

Findings:

- **Savings scale with overlap.** Cross-agent dedup is the dominant lever, and
  its value grows linearly with how much agents' observations overlap —
  exactly the regime (parallel exploration of one repository) where multi-agent
  systems are deployed.
- **Independent compilation plateaus.** Per-agent masking cannot remove the
  last `keep_recent` copies in each agent; with N agents holding the same
  payload, N copies survive every per-agent compiler. MACO keeps one.
- **Allocation properties hold** (unit-tested): the ledger never exceeds the
  global budget, surplus flows to high-weight agents, symmetric agents receive
  symmetric budgets, and regret signals strictly increase an agent's share.

Reproduce: `cargo test -p tokenopt-core --test orchestrator` and
`ctxc bench orchestrator --workers 8 --rounds 12 --shared-fraction 0.75 --global-budget 96000`.

## 6. Hypotheses for full evaluation (beyond this prototype)

- **H1 (cost):** on real multi-agent SWE traces, MACO reduces total input
  tokens ≥ 20% vs per-agent TokenOpt at equal task success, with the gap
  scaling in observation overlap (supported in simulation; needs live A/B with
  the `examples/e2e` harness extended to multi-agent runs).
- **H2 (quality):** water-filling beats uniform splits on task success under
  tight global budgets, because supervisors and high-regret agents stop being
  over-trimmed.
- **H3 (latency):** shared canonical copies improve provider prompt-cache hit
  rates (same canonical bytes across agents' prompts), compounding the token
  savings with cache discounts.
- **H4 (stability):** ARC converges — feedback boosts decay once rehydration
  stops, and the ledger does not oscillate (add decay term; prototype caps the
  boost at 3x but does not decay it yet).

## 7. Future work

- **Learned utility curves:** replace \(w_i \ln(1+b)\) with per-agent curves
  fit from sufficiency/rehydration history (contextual bandit over budgets).
- **Semantic dedup:** near-duplicate detection (MinHash / embeddings) to catch
  re-reads after small file edits; requires loss-aware previews.
- **KV-cache-aware packing:** order each agent's canonical shared blocks
  identically so providers' prefix caches hit across agents.
- **Budget markets:** let agents bid tokens for the next round from observed
  task progress, replacing fixed role multipliers with a clearing price.
- **Topology generalization:** the prototype assumes a star (orchestrator sees
  all agents); extend the ledger protocol to nested sub-orchestrators with
  budget delegation.

## 8. Positioning vs related work

| Work | Scope | What MACO adds |
|------|-------|----------------|
| LLMLingua-2, summarization | One prompt | Orchestrator-level allocation + shared store across prompts |
| MemGPT / MEM1 / external memory | One agent's memory | First-writer-wins canonical store shared by N agents |
| ACON retention policies | One transcript's blocks | Budgeting the *edges* (handoffs) by consumer contracts |
| Prompt caching (providers) | Identical prefixes, one model | Dedup of *observations* across distinct agents' contexts |
| SGCC (this repo, single-agent) | One transcript per call | Global ledger, cross-agent dedup, regret feedback |
