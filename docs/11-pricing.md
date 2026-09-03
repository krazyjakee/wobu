# 11 — Hunyuan3D pricing: what a mesh costs, and what we can honestly claim it costs

[08](08-providers.md)'s "Remaining unknowns" carries this flag:

> 🚩 Current pricing and free-credit allowance — the international `Query` response omits the
> `ResultCreditConsumed` field that the mainland one returns, so **we cannot read spend back
> from the API** and the cost estimate will have to be a local model of published prices.

This is the note that closes it ([#68](https://github.com/krazyjakee/wobu/issues/68)). It is a
record of what was **fetched and read**, not a design and not a recollection.

> ⚠️ **Sections 7 and 11 describe a spend ceiling that no longer exists.** The per-project ceiling
> and the cost estimate it enforced were removed: metering a local model of published prices tells
> the user about a number Wobu derived rather than the account that actually holds their money, and
> the two can disagree in either direction. What survives is the consent gate — Hunyuan3D charges
> per submitted job and never reports the amount, so `accept_cost` is the only honest gate — and
> everything below about *what Tencent actually charges*, which is still the best record we have.

> ✅ **Verified against Tencent's own documentation, 2026-08-01.** Every credit count, price
> and free-tier figure below was read off a Tencent page fetched on that date, and each row
> carries the page's own "Last updated" stamp so a re-check is a diff rather than a re-reading.
> Third-party aggregator prices were found, contradicted Tencent, and were discarded — see
> "Numbers that are circulating and are wrong". Nothing here was run against a live billing
> account, and claims that could not be settled from documentation are marked 🚩 with what it
> would take to settle them.

**The headline is better than the issue expected.** We assumed we would be guessing at a price.
We are not: the credit cost of a job is published, deterministic, and computable from the exact
parameters we already put in the request body. What we cannot see is the *rate* the account
converts credits at, and that is a bounded band rather than an open question. The right thing to
show the user is therefore an exact number and an inexact one, clearly separated — not one
number wearing a confidence it has not earned.

---

## 1. The flag is confirmed, and it is not a documentation gap

The international `Query` response genuinely has no credit field. Both endpoints, both doc
sites, and both SDK sources agree:

| Action | Host / version | Response fields | Credit field? |
| --- | --- | --- | --- |
| `QueryHunyuanTo3DProJob` **international** | `hunyuan.intl.tencentcloudapi.com` `2023-09-01` | `Status` `ErrorCode` `ErrorMessage` `ResultFile3Ds` `RequestId` | **no** |
| `QueryHunyuanTo3DProJob` international `ai3d` | `ai3d.intl.tencentcloudapi.com` `2025-05-13` | same five | **no** |
| `QueryHunyuanTo3DProJob` **mainland** | `ai3d.tencentcloudapi.com` `2025-05-13` | the five, plus `ResultCreditDetails` and `ResultCreditConsumed` | **yes** |
| `SubmitHunyuanTo3DProJob` either site | — | `JobId` `RequestId` | **no** |

Sources: international [1284/75541](https://www.tencentcloud.com/document/product/1284/75541)
(page stamped `2026-05-27 12:03:00`), mainland
[1804/123448](https://cloud.tencent.com/document/api/1804/123448) (stamped `2026-05-11 01:07:52`).
Corroborated in SDK source, which is where a doc omission would show up as a real one:
`tencentcloud-sdk-go-intl-en/tencentcloud/hunyuan/v20230901/models.go` and the `ai3d/v20250513`
sibling both contain **zero** case-insensitive matches for "credit"; the mainland
`tencentcloud-sdk-go/tencentcloud/ai3d/v20250513/models.go` contains exactly two,
`ResultCreditDetails *string` and `ResultCreditConsumed *float64`.

Our own reader is consistent with this and reads nothing it should not:
`tencent/wire.rs::progress` takes `Status`, `ErrorCode`, `ErrorMessage` and `ResultFile3Ds` and
that is the whole of it. So this is not a field we are failing to parse.

**The useful detail is when the mainland gained it.** Its
[update history](https://cloud.tencent.com/document/product/1804/120839) dates the addition to
release 12, **2026-03-17**: *"新增出参：ResultCreditDetails, ResultCreditConsumed"*. The
international product has no equivalent entry. So this is a recent mainland addition that
international has not picked up — which means it may yet arrive, and that is a cheap thing to
re-check rather than a permanent architectural fact. Nothing should be built that would be hard
to unwind if it appears.

### The three fallbacks, and why none of them is the answer

- **Call the mainland endpoint instead.** Only works for a mainland account, and
  [08](08-providers.md) already establishes that a non-mainland account is the case we support.
- **Read the billing API.** `DescribeBillDetail`, `DescribeBillResourceSummary` and
  `DescribeAccountBalance` all exist on `billing.intl.tencentcloudapi.com` `2018-07-09`. They
  are month-scoped and resource-granularity, not per-request, and the previous day's bill is
  generated around 08:00 the next morning — so they cannot tell a user what the mesh they are
  looking at cost. `DescribeAccountBalance` is near-real-time but reports a whole-account
  balance that every other Tencent resource also moves, and this product deducts credits rather
  than balance, so a before/after diff would frequently read zero.
  🚩 The near-real-time diff was reasoned about, not tried.
- 🚩 **Worse than useless, actually: the permissions.** The minimum policy for those reads is
  `QcloudFinanceBillReadOnlyAccess`, an *account-level financial* grant. [08](08-providers.md)
  already steers users toward a CAM sub-account key scoped to the 3D service precisely to limit
  blast radius; asking the same key to read the billing API would hand a desktop binary the
  user's entire billing history to show them a number we can compute locally. **Do not do this.**

So: a local model it is. The rest of this document is that model, and the honesty it requires.

---

## 2. What we can know exactly: credits

International Hunyuan3D does not bill per call. It bills **credits**, and the credit cost of a
Pro job is a published function of parameters we choose and send:

**Base, by `GenerateType`** — pick exactly one:

| `GenerateType` | Credits | On 3.1? |
| --- | --- | --- |
| `Geometry` — untextured white model | **15** | yes |
| `Normal` — geometry and texture | **25** | yes |
| `Sketch` | 25 | no — 3.0 only |
| `LowPoly` | 30 | no — 3.0 only |

**Add-ons, cumulative** — Tencent's wording is *"Can be used after selecting the task generation
type. The following parameters allow multiple options and will accumulate credits consumption
based on additional usage."*

| Parameter | Credits |
| --- | --- |
| `MultiViewImages` | **+10** |
| `EnablePBR` | **+10** |
| custom `FaceCount` | **+10** |
| `ResultFormat` | not billed internationally — the parameter does not exist there |

**Model version is free.** Verbatim: *"Selecting version 3.0 or 3.1 from the [Model] parameter
will not consume extra credits."* This confirms as fact what `tencent/mod.rs` already asserts in
a doc comment — that an omitted `Model` is "a silently older reconstruction **at the same
price**". That sentence was a guess when it was written and it is now checked.

Source: [Tencent HY 3D Global Billing Overview,
1284/75281](https://www.tencentcloud.com/document/product/1284/75281), page stamped
**`2026-05-08 19:51:49`**, fetched 2026-08-01.

**Failed jobs are free.** *"Failed tasks do not deduct credits (no deduction for any failure,
including failures caused by risk control policy)."* This matters for `wobu-jobs`: a `Billed`
verdict on a `Status: FAIL` poll is, on this provider, wrong — the retry policy may retry it.
🚩 Whether a job cancelled mid-run counts as failed or as completed is not documented, and
`MeshUsage` deliberately counts from the moment a `JobId` exists, which is the conservative
reading.

### Every job wobu can send, priced in credits

Model 3.1 is our default, so `Sketch` and `LowPoly` are out of reach and the base is 15 or 25:

| Job | Credits |
| --- | --- |
| `Geometry`, single image | 15 |
| `Normal`, single image | 25 |
| `Geometry`, turnaround (multi-view) | 25 |
| **`Normal`, turnaround — the Turnaround preset's job** | **35** |
| `Normal`, turnaround, custom `FaceCount` | **45** |
| `Normal`, turnaround, custom `FaceCount`, `EnablePBR` | **55** |

**A job is between 15 and 55 credits — a 3.7x spread.** Remember that number; section 7 turns on
it.

---

## 3. Default face count is omitted; custom face count costs 10 credits

Tencent's submit API documents `FaceCount` as **optional**, with a default of `500000`. Its
billing table describes the add-on more precisely than the surrounding prose: **"Generate 3D
models with custom face counts"** costs 10 additional credits.

`tencent/wire.rs` therefore omits `FaceCount` at exactly the provider default:

```rust
"GenerateType": request.generate_type.as_str(),
"EnablePBR": request.enable_pbr,
// "FaceCount" is inserted only when request.face_count != 500_000.
```

This does not require Wobu to guess how Tencent's billing implementation treats an explicitly
sent default. Omission and `FaceCount: 500000` have the same documented mesh semantics, while
omission is unambiguously the ordinary default request. Every value from 3000 through 1500000
other than 500000 is still sent, so a caller asking for a custom triangle budget gets precisely
that budget and the cost model includes the documented +10 credits.

A real-account A/B run could still reveal whether Tencent also waives the add-on when 500000 is
sent explicitly, but it is no longer required for Wobu's behaviour or pricing: Wobu does not
send that ambiguous request.

---

## 4. Credits to money — the part we cannot see

Credits are bought, and the rate depends on how. Settlement runs **free package → prepaid
package → postpaid**, so a given job's real cost depends on which pool it lands in, and wobu
cannot see any of the three.

| Rate | USD/credit | Verified | Page stamp |
| --- | --- | --- | --- |
| Postpaid (pay-as-you-go), settled daily | **0.0200** | 2026-08-01 | `2026-05-08 19:51:49` |
| Prepaid pack, 1,000 credits — USD 15 | **0.0150** | 2026-08-01 | `2026-05-08 19:51:49` |
| Prepaid pack, 10,000 credits — USD 145 | **0.0145** | 2026-08-01 | `2026-05-08 19:51:49` |
| Prepaid pack, 50,000 credits — USD 700 | **0.0140** | 2026-08-01 | `2026-05-08 19:51:49` |
| Prepaid pack, 100,000 credits — USD 1,350 | **0.0135** | 2026-08-01 | `2026-05-08 19:51:49` |
| Free grant on activation — 200 credits, one-time, 1-year validity | **0.0000** | 2026-08-01 | `2026-05-08 19:51:49` |

Prepaid credits expire after one year; unused packs are refundable within seven days; packs only
offset usage generated after purchase.

**The band is 0.0135 to 0.0200 — a factor of 1.48, not a factor of three.** The volume ladder
itself is shallow (10% across the whole range); nearly all of the spread is prepaid versus
postpaid, which is 32.5%. This is the single most reassuring fact in the document: the money
figure is uncertain within a range we can state, and we can state both ends.

So the Turnaround preset's job:

| | Credits | Postpaid | Cheapest prepaid | Band |
| --- | --- | --- | --- | --- |
| `Normal` + multi-view, default face count | 35 | USD 0.70 | USD 0.47 | **0.47 – 0.70** |
| `Normal` + multi-view, custom face count | 45 | USD 0.90 | USD 0.61 | **0.61 – 0.90** |

For the default scenario the ceiling exists for — 200 meshes unattended — that is **7,000
credits, USD 95 to 140**, and the 200 free credits cover **five** complete jobs.

### Concurrency is where the money actually is

Default concurrency is 3 for Pro (which `wobu-jobs::Queue` already defaults to, naming
Hunyuan3D as the reason) and 1 for Express. Additional concurrency is sold at **USD 5,000 per
concurrent slot per month** — roughly 5,500 meshes' worth. Nothing in wobu should ever present
raising concurrency as a setting. It is not a slider; it is an enterprise contract, and the
queue's cap of 3 is the correct and permanent answer.

---

## 5. International and mainland do not agree, and it is not the exchange rate

They are separate products with separately maintained price tables, and they differ in the
credit counts as well as the rate:

| | International (product 1284) | Mainland (product 1804) |
| --- | --- | --- |
| `Normal` | **25** credits | **20** credits |
| `LowPoly` | **30** credits | **25** credits |
| `Geometry` | 15 credits | 15 credits |
| `Sketch` | 25 credits | 25 credits |
| `MultiViewImages` / `EnablePBR` / `FaceCount` | +10 / +10 / +10 | +10 / +10 / +10 |
| `ResultFormat` | parameter absent | +5 credits |
| Postpaid rate | **USD 0.0200** / credit | **CNY 0.12** / credit |
| Prepaid ladder | USD 0.0150 → 0.0135 | CNY 0.10 → 0.09 |
| Free grant | **200** credits, **granted automatically** on activation | **100** credits, **must be claimed manually** in the console |
| Page stamp | `2026-05-08 19:51:49` | `2026-07-24 18:14:00` |

Two things follow, and both are traps.

**Never convert one into the other.** The rates are set independently, not by an FX peg. USD
0.02 and CNY 0.12 per credit are equal only at exactly **6.00 CNY per USD**; the `Normal` job at
25 credits versus 20 credits is equal only at **4.80 CNY per USD**. Two different implied rates
out of the same pair of tables is proof there is no conversion happening. A mainland figure
converted at any real rate would understate the international cost, and a reader who found the
CNY page first would conclude a textured mesh is meaningfully cheaper than it is.
🚩 No FX rate was looked up for this document, deliberately — the point is that none applies.

**The mainland page is 2.5 months fresher.** That asymmetry is worth watching rather than
explaining: it may mean the international table is simply lagging a change that has already
landed in China, in which case international `Normal` may drop from 25 to 20. It is not
something to price in, but it *is* a reason to treat the international stamp as the one to
re-check.

🚩 **One mainland-only warning we could not evaluate for international.** The mainland billing
page opens with a migration notice: Hunyuan model services are moving to **TokenHub**, and after
migration the old platform *"will stop supporting new purchases of model services"*. If the same
migration reaches international, a new signup may be unable to buy credits on the path this
document describes, and TokenHub's pricing is unknown. No equivalent notice appears on the
international page. This is the largest single risk to everything above.

---

## 6. Numbers that are circulating and are wrong

Searching for this pricing surfaces confident third-party figures that contradict Tencent's own
table. Recording them so nobody re-derives them:

| Claim, seen on aggregator sites | What is actually true |
| --- | --- |
| "USD 0.02 per generation" | USD 0.02 is the postpaid price per **credit**. The cheapest job is 15 credits and the dearest 55. Off by at least tenfold, and up to 55x. |
| "Pro is 60 credits, Rapid is 35" | Matches no row in Tencent's published table. |
| "20 free generations per day" | That is the consumer **Hunyuan 3D Studio** web app, not the cloud API. The API grant is 200 credits, once. |

The first of these is the dangerous one: it is off in the direction that makes a spend ceiling
useless, and it is the number a user is most likely to arrive with.

---

## 7. What the ceiling should count: credits — not jobs, and not money

The scenario is a Turnaround loop left running against 200 entities overnight. The question is
what unit stops it.

**Not money.** A money ceiling is enforced against a number we derived; it inherits every
uncertainty in section 4. It is the number the user *thinks* in,
so it must be shown — but a limit that halts real work should not be computed from a table
nobody has confirmed against the account it is protecting.

**Not jobs either, and this is the part worth arguing.** A job count is exactly knowable, which
is the whole appeal, and `MeshUsage::billed_jobs` already exists and is right to. But section 2
showed a job is **15 to 55 credits — a 3.7x spread**. A ceiling of "200 jobs" authorises
somewhere between 3,000 and 11,000 credits, which is USD 40 to USD 220. That is a *worse* spread
than the money estimate it was supposed to protect us from, and it is worse in the silent
direction: the user who sets 200 jobs while generating untextured `Geometry` previews, then
switches the preset to textured multi-view with PBR, has quietly tripled the ceiling without
touching it.

**Credits.** They are the provider's own unit; they are computed from parameters we chose and
put in the body, so the count is arithmetic on our own request rather than a reading of theirs;
they collapse the 3.7x job spread to zero; and they are the one number a Tencent support ticket
would recognise. The only step credits do not cover is credits-to-money, which is section 4's
1.48x band — and that band is *displayed*, not enforced.

So:

- **Enforce on credits.** `MeshUsage` should carry `billed_credits` alongside `billed_jobs`,
  computed at submit time from the same `MeshRequest` that built the body. Keep `billed_jobs`:
  it is what the concurrency cap and the free-grant arithmetic are counted in, and it is the
  honest denominator for "5 free meshes".
- **Display money**, as a range, always labelled, never as the thing that stopped the run.
- **The stop message names the exact unit**: "stopped after 9,000 credits" is a sentence Tencent
  would agree with. "Stopped after USD 180" is one only we believe.

🚩 This is a recommendation, not an implementation. `MeshUsage` today counts jobs and its test is
named `usage_counts_jobs_because_there_is_no_credit_figure_to_read` — which was the right
conclusion from what [08](08-providers.md) knew, and is now superseded by the finding that the
credit figure does not need reading because it can be computed. Changing it is M6/M8 work with
the ceiling in front of it, not a spike edit; see section 11.

---

## 8. Free credits push the error in the safe direction, and corrode trust anyway

**Safe.** The 200-credit grant is applied first, so if the model charges for jobs the account
got free, wobu **over**-reports spend. An over-reporting ceiling trips early: the run stops
before the user's real limit, they have spent less than we said, and the failure mode is an
interruption rather than a bill. That is the direction to be wrong in, and it is an argument for
**not** modelling the free grant at all — assume every job is paid, and let the first five be a
pleasant surprise.

**The trust cost is real and should be paid up front.** Those first five jobs are exactly when
the user calibrates how much to believe our number. They see wobu claim USD 3.50 while the
console says zero, learn the figure is pessimistic, and then the grant runs out and the figure
becomes accurate — right before the 200-mesh run. The fix is not to model the grant; it is to
say so in onboarding: *the first 200 credits are free, and wobu counts them as if you paid.*

**We cannot detect exhaustion**, which settles it. Wobu would have to have observed every job
ever run on that account — including ones from the console, from other tools, and from before
the app was installed — to know where the free pool stands. Any attempt would be wrong on a
reinstall and wrong on a shared project folder.

🚩 One mainland-documented behaviour we could not confirm for international, and it is not a cost
problem but a runtime one: mainland states that when free credits are exhausted, calls **error**
rather than falling through to postpaid, unless postpaid has been explicitly enabled in the
console. The international page describes automatic settlement order and no such requirement. If
international behaves the same way, wobu's sixth mesh on a fresh account fails with a billing
error, and that deserves a specific message and a console link — not a generic provider failure.
This is worth checking on a real account.

---

## 9. Currency: display the unit, never convert

The international product publishes in **USD** and only USD — the columns are literally headed
"Price (USD)". The mainland product publishes in **CNY** and only CNY. Neither purchase page
states what currency the account is actually charged in.

Three rules follow:

1. **Write "USD 0.90", never "$0.90".** The bare glyph is ambiguous across at least a dozen
   currencies, and a user whose card settles in something else needs to see which one we meant
   in order to know the number is not theirs.
2. **Never convert, and never show a converted figure.** A conversion needs a rate; a rate needs
   a date; and wobu is local-first with no business calling an FX service. A converted number
   would stack a second undated uncertainty on top of an unverified price, and it would be wrong
   in a way with no visible cause.
3. **Never take a figure from the other site.** Section 5 shows the two tables are not
   translations of each other. A CNY price is not this product's price in another currency; it
   is a different product's price.

🚩 What currency a non-mainland Tencent Cloud account is billed in — and whether it varies by
signup region or payment method — was not established. If it commonly is not USD, the honest
sentence in section 10 needs a clause saying so.

---

## 10. What the user sees

Two numbers, visibly different in kind. The credit count is a fact and should look like one; the
money is ours and should say whose it is.

**Inline, on the Generate button and in the queue row** — the compact form:

> **35 credits · about USD 0.47–0.70**

**The sentence, wherever a run is authorised** — the ceiling dialog, and the confirmation before
a batch:

> **About USD 0.47–0.70 for this mesh.** The 35 credits are exact — that is what Tencent's
> published table charges for a textured multi-view job. The money is our own estimate from
> prices we last checked on 1 August 2026, and the range is because we cannot see whether your
> account is on pay-as-you-go or a prepaid credit pack. Wobu cannot read your Tencent bill.

If only one line fits, it is this one:

> About USD 0.47–0.70 — our own figure from prices we last checked on 1 August 2026. Wobu cannot
> confirm it against your Tencent account.

Three deliberate choices in that wording:

- **A range, not a midpoint.** "About USD 0.59" is a worse promise than "USD 0.47–0.70" and no
  shorter. The range *is* the honesty; collapsing it to a point invents a precision we would
  then have to apologise for in the next sentence.
- **"Our own figure" before "cannot confirm".** Ownership first, limitation second. The reverse
  order reads as a disclaimer to be skimmed past; this order tells the user who to blame, which
  is the fact they need.
- **The date is in the sentence, not in a tooltip.** A provenance that requires a hover is a
  provenance most users never see, and this number's entire claim to being honest is its
  provenance.

**Where it does not go:** it does not go only in Settings. `Settings.tsx` already models the
right pattern for the key-check cost — a `prov-cost` line sitting directly beside the button
that spends the money — and this belongs in the same place, next to the action, not in a page
the user visits once.

**Onboarding gets one extra sentence**, per section 8:

> Your first 200 credits are free — about five meshes. Wobu counts them as if you paid, so the
> first few estimates will read high.

---

## 11. Making staleness loud

This table can never be reconciled against the API, so the only defence is that a reader can
tell at a glance how old it is. Four mechanisms, in increasing order of how much they help:

1. **A verified date on every row, not one at the top of the file.** A partial re-check is the
   normal case — somebody confirms the postpaid rate and does not re-read the credit table — and
   a single document-level date would either lie about the rows nobody looked at or be reset by
   someone who only looked at one.
2. **Record Tencent's own page stamp next to ours.** Every table above carries the
   `Last updated` string the source page displays. This turns re-verification from "read the
   whole page again and hope you notice" into "fetch the page and compare one string", which is
   a check somebody will actually perform. The stamps to beat are
   **`2026-05-08 19:51:49`** (international billing) and **`2026-07-24 18:14:00`** (mainland).
3. **A recheck horizon written as a date.** These prices should be re-read by
   **2026-11-01**. Written as a literal date rather than "every 90 days" so that it is a date in
   the past when it lapses, which is something a reader notices and an interval is not.
4. **Withdraw the number rather than degrading its label.** The strongest mechanism, and the one
   that belongs in code: if the price table is older than its horizon, the UI should stop showing
   money and show credits alone. Credits do not go stale — they come from a table we can check
   against our own request, and a stale credit count is still correct arithmetic. Money does. A
   figure that survives indefinitely with an increasingly old date attached is exactly the silent
   staleness this section exists to prevent; a figure that disappears is not.

**If a price table is ever written in Rust, this is the shape it needs.** Not a `f64` per job,
but a value that cannot be rendered without its provenance:

```
Estimate { credits: u32, per_credit: RangeInclusive<Money>, verified_on: Date, source: &str }
```

— with no accessor returning the money alone. It is the same trick `GeneratedMesh` uses by
having no field for a result URL: the property is enforced by there being nothing to call, which
is checkable, rather than by everyone remembering, which is not.

---

## 12. What could not be established

- 🚩 **Whether TokenHub migration reaches the international product**, and what it prices at.
  Section 5. The largest risk to this entire document, and the only one that could invalidate it
  wholesale rather than by a percentage.
- 🚩 **Whether international hard-errors when free credits run out**, as mainland documents.
  Section 8. A runtime-error mapping, not a cost question.
- 🚩 **What currency a non-mainland account is actually charged in.** Section 9.
- 🚩 **Whether a cancelled-mid-run job is billed.** "Failed tasks do not deduct credits" is
  documented; cancellation is not addressed. `MeshUsage` assumes billed, which is conservative.
- 🚩 **Whether `DescribeAccountBalance` moves measurably across one 3D job.** Reasoned about in
  section 1, not tried — and given the CAM blast radius, not worth trying.
- 🚩 **Every figure here is documentation, not an invoice.** Nothing in this file has been
  checked against a real Tencent bill. The first user who generates a mesh and looks at their
  console knows more than this document does, and that observation should be written down here
  when it happens.

---

## 13. What this changes

**Only default `FaceCount` wire presence was edited in the adapter.** No price table was added
to `wobu-imagine/src/tencent/`, on purpose. A price is a fact about a vendor's commercial terms
with a shelf life measured in months; the adapter is a fact about a wire protocol. Putting the
table in `tencent/` would place the most volatile thing we know inside the code most protected
by tests, and the tests would not be able to tell a stale price from a fresh one — the one
property that matters. When it does need to be code, it belongs with the spend ceiling in M6,
shaped as section 11 describes, and this file is what it is transcribed from.

Consequences worth carrying forward:

| Where | What |
| --- | --- |
| [09](09-roadmap.md) M6 — cost estimation and the spend ceiling | The ceiling's unit is **credits** (section 7). Money is displayed as a range, never enforced. |
| `wobu-imagine/src/mesh.rs` — `MeshUsage` | Add `billed_credits` beside `billed_jobs`, computed at submit from the same `MeshRequest` that builds the body. The doc comment saying a per-call cost must be guessed is now half wrong: the credits are exact, only the rate is not. |
| `wobu-imagine/src/tencent/wire.rs` | `FaceCount` is omitted at Tencent's documented 500000 default and sent for every custom value. |
| `wobu-imagine/src/tencent/mod.rs` | "an omitted `Model` is a silently older reconstruction at the same price" is ✅ confirmed verbatim by Tencent. No change needed — recorded so nobody re-verifies it. |
| `wobu-jobs` retry policy | Failed Hunyuan3D jobs are explicitly **not** charged, so a `Status: FAIL` poll is a candidate for retry rather than a billed failure. This contradicts the general rule and needs an exception, or the queue will refuse to retry work that cost nothing. |
| [08](08-providers.md) "Remaining unknowns" | The pricing 🚩 can be replaced with a pointer here. The `ResultCreditConsumed` finding it records is confirmed correct. |
| Onboarding | Two sentences: the free grant is five default meshes and we count it as paid; concurrency above 3 costs USD 5,000/month and is not a setting. |

---

## Sources

Every page below was fetched **2026-08-01**; the stamp is the page's own.

| Page | Stamp |
| --- | --- |
| [Tencent HY 3D Global — Billing Overview (1284/75281)](https://www.tencentcloud.com/document/product/1284/75281) — the international price table | `2026-05-08 19:51:49` |
| [Purchase Method (1284/75283)](https://www.tencentcloud.com/document/product/1284/75283) — console paths, balance | `2026-03-03 10:32:34` |
| [`SubmitHunyuanTo3DProJob` (1284/75540)](https://www.tencentcloud.com/document/product/1284/75540) — confirms host `hunyuan.intl.tencentcloudapi.com`, version `2023-09-01`, no `ResultFormat` | `2026-05-27 12:02:55` |
| [`QueryHunyuanTo3DProJob` (1284/75541)](https://www.tencentcloud.com/document/product/1284/75541) — the five response fields | `2026-05-27 12:03:00` |
| [混元生3D 计费概述 (1804/123461)](https://cloud.tencent.com/document/product/1804/123461) — the mainland price table | `2026-07-24 18:14:00` |
| [`QueryHunyuanTo3DProJob` mainland (1804/123448)](https://cloud.tencent.com/document/api/1804/123448) — `ResultCreditConsumed`, `ResultCreditDetails` | `2026-05-11 01:07:52` |
| [混元生3D update history (1804/120839)](https://cloud.tencent.com/document/product/1804/120839) — credit fields added 2026-03-17 | — |
| [计费概述（个数）(1804/120700)](https://cloud.tencent.com/document/product/1804/120700) — the **legacy per-generation** mainland scheme, explicitly *not* applicable to Pro or Rapid. Do not read this one by mistake. | `2025-09-16` |

Console, for a user checking their own account: activate and claim at
[console.tencentcloud.com/hunyuan](https://console.tencentcloud.com/hunyuan), buy packs at
[buy.tencentcloud.com/hunyuan](https://buy.tencentcloud.com/hunyuan), and read the remaining
balance under **Resource Package Management** in the console's left navigation. That console
page is the only place the true, account-specific figure exists — this document is a model of
it, and says so everywhere it is quoted.
