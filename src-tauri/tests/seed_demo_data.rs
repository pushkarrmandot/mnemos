//! Populates `$MNEMOS_HOME` with a realistic, fully-worked-through set of
//! demo data: five projects, ~29 conversations, transcripts, summaries,
//! extraction, and project memory docs. Built for later use as
//! demo/screenshot data — not exercised in CI, and it never touches the
//! real `~/Mnemos/` data root (everything goes through
//! `fs::paths::data_root`'s `$MNEMOS_HOME` override).
//!
//! Run with:
//! `MNEMOS_HOME=/path/to/scratch-dir cargo test --manifest-path
//! src-tauri/Cargo.toml --test seed_demo_data -- --ignored --nocapture`

use std::time::{SystemTime, UNIX_EPOCH};

use mnemos_tauri_lib::db::models::{
    ConversationStatus, ExtractionBundle, NewActionItem, NewConversation, NewDecision,
    NewOpenQuestion, NewProject,
};
use mnemos_tauri_lib::db::service::{SqliteStorageService, StorageService};
use mnemos_tauri_lib::db::{self};
use mnemos_tauri_lib::fs::paths;

const DAY_S: i64 = 86_400;

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_secs() as i64
}

// TODO(seed-data-follow-up): these still pass real names (e.g. "Pushkar")
// where production now writes `is_self: true` + the onboarded name instead
// of a "You" sentinel — see product_docs plan for the *_is_self split.
// Deliberately left as a follow-up pass, not part of this schema change; the
// `false` below is a compile-only placeholder, not a claim about who these
// items belong to.
fn ai(text: &str, assignee: Option<&str>, due: Option<&str>, ts: Option<i64>) -> NewActionItem {
    NewActionItem {
        text: text.to_string(),
        assignee_hint: assignee.map(str::to_string),
        assignee_is_self: false,
        due_hint: due.map(str::to_string),
        source_ts: ts,
    }
}

fn dec(
    statement: &str,
    quote: Option<&str>,
    decided_by: Option<&str>,
    ts: Option<i64>,
) -> NewDecision {
    NewDecision {
        statement: statement.to_string(),
        quote: quote.map(str::to_string),
        decided_by_hint: decided_by.map(str::to_string),
        decided_by_is_self: false,
        source_ts: ts,
    }
}

fn oq(question: &str, raised_by: Option<&str>, ts: Option<i64>) -> NewOpenQuestion {
    NewOpenQuestion {
        question: question.to_string(),
        raised_by_hint: raised_by.map(str::to_string),
        raised_by_is_self: false,
        source_ts: ts,
    }
}

/// Lays out a list of `(source, text)` lines evenly across `duration_ms`,
/// producing the exact transcript-turn shape `write_transcript` expects.
/// Returns the turn JSON values alongside the `ts_start_ms` each line
/// landed at, so callers can wire `source_ts` on extraction rows back to a
/// real turn instead of guessing a number.
fn build_turns(lines: &[(&str, &str)], duration_ms: i64) -> (Vec<serde_json::Value>, Vec<i64>) {
    let n = lines.len() as i64;
    let step = (duration_ms / n).max(1000);
    let mut turns = Vec::with_capacity(lines.len());
    let mut starts = Vec::with_capacity(lines.len());
    for (i, (source, text)) in lines.iter().enumerate() {
        let start = i as i64 * step;
        let end = (start + step - 400).min(duration_ms).max(start + 200);
        let speaker_label = if *source == "mic" { "You" } else { "Them" };
        starts.push(start);
        turns.push(serde_json::json!({
            "text": text,
            "ts_start_ms": start,
            "ts_end_ms": end,
            "source": source,
            "speaker_label": speaker_label,
            "speaker_label_source": "source_file",
            "contact_id": null,
        }));
    }
    (turns, starts)
}

struct ConvSpec {
    title: &'static str,
    days_ago: i64,
    hour_offset_s: i64,
    duration_s: i64,
    lines: Vec<(&'static str, &'static str)>,
    summary: String,
    action_items: Vec<NewActionItem>,
    decisions: Vec<NewDecision>,
    open_questions: Vec<NewOpenQuestion>,
}

/// Creates the conversation, marks it Ready, and writes transcript,
/// summary, and extraction. Returns `ended_at` so the caller can track the
/// project's latest activity for its memory doc's `last_refresh_at`.
async fn seed_conv(
    storage: &dyn StorageService,
    project_id: &str,
    now: i64,
    spec: ConvSpec,
) -> Result<i64, Box<dyn std::error::Error>> {
    let started_at = now - spec.days_ago * DAY_S + spec.hour_offset_s;
    let ended_at = started_at + spec.duration_s;
    let duration_ms = spec.duration_s * 1000;

    let conv = storage
        .insert_conversation(NewConversation {
            project_id: Some(project_id.to_string()),
            title: spec.title.to_string(),
            started_at,
            runner_id: None,
        })
        .await?;

    storage
        .update_conversation_status(
            &conv.id,
            ConversationStatus::Ready,
            Some(ended_at),
            Some(spec.duration_s),
        )
        .await?;

    let (turns, _starts) = build_turns(&spec.lines, duration_ms);
    let transcript = serde_json::json!({
        "schema_version": 1,
        "conversation_id": conv.id,
        "duration_ms": duration_ms,
        "turns": turns,
    });
    storage.write_transcript(&conv.id, &transcript).await?;
    storage.write_summary(&conv.id, &spec.summary).await?;
    storage
        .bulk_insert_extraction(
            &conv.id,
            ExtractionBundle {
                action_items: spec.action_items,
                decisions: spec.decisions,
                open_questions: spec.open_questions,
                bookmarks: vec![],
            },
        )
        .await?;

    Ok(ended_at)
}

#[tokio::test]
#[ignore]
async fn seed_demo_data() -> Result<(), Box<dyn std::error::Error>> {
    let mnemos_home = std::env::var("MNEMOS_HOME").expect(
        "set MNEMOS_HOME to a scratch directory before running this seed — never run it \
         without an override, it writes real data",
    );
    eprintln!("seeding demo data into MNEMOS_HOME={mnemos_home}");

    let db_path = paths::db_path()?;
    assert!(
        db_path.starts_with(&mnemos_home),
        "refusing to seed: db_path {db_path:?} is not under MNEMOS_HOME {mnemos_home:?}"
    );

    let pools = db::init(&db_path).await?;
    let storage = SqliteStorageService::new(pools);

    storage
        .set_setting("onboarding.has_onboarded", serde_json::json!(true))
        .await?;
    storage
        .set_setting("onboarding.user_first_name", serde_json::json!("Pushkar"))
        .await?;

    let now = now_unix();

    // ---------------------------------------------------------------
    // Project 1: Series A
    // ---------------------------------------------------------------
    let series_a = storage
        .create_project(NewProject {
            name: "Series A".to_string(),
            description: Some(
                "Fundraising process and investor relations for the Series A round.".to_string(),
            ),
        })
        .await?;

    let mut series_a_last = 0i64;

    {
        let lines = vec![
            ("mic", "Thanks for making time. Wanted to walk through the metrics packet you sent over and answer anything that's unclear."),
            ("system", "Appreciated. The retention curve is the piece we keep coming back to — can you say more about the dip in month three?"),
            ("mic", "That's the cohort that onboarded during the pricing change in March. We've since fixed the onboarding flow for new cohorts and the dip hasn't repeated."),
            ("system", "Good to hear. We'll circle back once the partner group has reviewed."),
        ];
        let (_, starts) = build_turns(&lines, 360_000);
        let ended = seed_conv(&storage, &series_a.id, now, ConvSpec {
            title: "Diligence call — Redpoint",
            days_ago: 58,
            hour_offset_s: 3600,
            duration_s: 360,
            lines,
            summary: "Redpoint's diligence call focused on the month-three retention dip visible in the metrics packet, which we attributed to the March pricing-change cohort. No new material was requested; they'll follow up after partner review.".to_string(),
            action_items: vec![ai("Send Redpoint the updated cohort retention chart excluding the March pricing-change cohort", Some("Pushkar"), Some("this week"), Some(starts[2]))],
            decisions: vec![dec("Hold off on sending additional cohort data until Redpoint's partner group responds", None, None, Some(starts[3]))],
            open_questions: vec![oq("Whether Redpoint's partner group will want a reference call with an existing customer before moving forward", Some("Redpoint"), Some(starts[3]))],
        }).await?;
        series_a_last = series_a_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Ran through the three references. Overall very positive — one flagged concern worth noting."),
            ("system", "What was it?"),
            ("mic", "One customer mentioned support response times slipped during our busiest month. Nothing structural, but worth having an answer ready."),
            ("system", "Makes sense. Let's fold that into the FAQ doc for the partner meeting."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &series_a.id, now, ConvSpec {
            title: "Reference check debrief",
            days_ago: 45,
            hour_offset_s: 7200,
            duration_s: 300,
            lines,
            summary: "Reference checks came back largely positive. One customer flagged slower support response times during a peak month — not a structural issue, but worth addressing proactively before it's raised as a question.".to_string(),
            action_items: vec![ai("Add a support-responsiveness talking point to the investor FAQ doc", Some("Pushkar"), Some("before partner meeting"), Some(starts[2]))],
            decisions: vec![dec("Include the support-response concern proactively in the FAQ rather than waiting to be asked", Some("Let's fold that into the FAQ doc for the partner meeting."), Some("Pushkar"), Some(starts[3]))],
            open_questions: vec![],
        }).await?;
        series_a_last = series_a_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Lawyer flagged two clauses — the pro-rata rights and the information rights schedule."),
            ("system", "Pro-rata is standard at this stage, I wouldn't push back there. The information rights schedule is broader than typical though — worth a redline."),
            ("mic", "Agreed. I'll send the redline tonight."),
            ("system", "Also double check the board observer seat language — make sure it's non-voting, explicitly."),
            ("mic", "Will confirm and get back to you."),
        ];
        let (_, starts) = build_turns(&lines, 420_000);
        let ended = seed_conv(&storage, &series_a.id, now, ConvSpec {
            title: "Term sheet review",
            days_ago: 30,
            hour_offset_s: 1800,
            duration_s: 420,
            lines,
            summary: "Reviewed the term sheet with counsel. Pro-rata rights are standard and not worth contesting. The information rights schedule is broader than typical for this stage and needs a redline. Also flagged the board observer seat language for an explicit non-voting confirmation.".to_string(),
            action_items: vec![
                ai("Send redlined information rights schedule to counsel", Some("Pushkar"), Some("tonight"), Some(starts[2])),
                ai("Confirm board observer seat is explicitly non-voting in the term sheet", Some("Pushkar"), None, Some(starts[4])),
            ],
            decisions: vec![dec("Accept the pro-rata rights clause as drafted; push back only on the information rights schedule", None, Some("Pushkar"), Some(starts[2]))],
            open_questions: vec![oq("Whether the lead will accept a narrower information rights schedule without re-opening other terms", None, Some(starts[1]))],
        }).await?;
        series_a_last = series_a_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Deck's mostly done. Want to sanity check the burn slide before I send it out."),
            ("system", "Numbers look right. I'd add a line on the hiring slowdown so it doesn't read as a surprise later."),
            ("mic", "Good call, I'll add that."),
        ];
        let (_, starts) = build_turns(&lines, 360_000);
        let ended = seed_conv(&storage, &series_a.id, now, ConvSpec {
            title: "Board prep",
            days_ago: 12,
            hour_offset_s: 5400,
            duration_s: 360,
            lines,
            summary: "Quick review of the board deck ahead of the meeting. Burn numbers check out; adding a short note on the Q3 hiring slowdown so it reads as a deliberate choice rather than a surprise.".to_string(),
            action_items: vec![ai("Add a note on the Q3 hiring slowdown to the board deck burn slide", Some("Pushkar"), None, Some(starts[1]))],
            decisions: vec![],
            open_questions: vec![oq("Whether to include the Aurora launch numbers before or after the September investor update", None, Some(starts[2]))],
        }).await?;
        series_a_last = series_a_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Draft's ready. Wanted to check the framing on the Aurora launch numbers before it goes out."),
            ("system", "Framing's fine. I'd lead with retention rather than signups though — that's the number people ask about."),
            ("mic", "Makes sense, I'll reorder the sections."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &series_a.id, now, ConvSpec {
            title: "Investor update — Sept",
            days_ago: 3,
            hour_offset_s: 2400,
            duration_s: 300,
            lines,
            summary: "Reviewed the September investor update draft. The Aurora launch numbers are in good shape; the main note was to lead with retention rather than signup counts, since that's what investors tend to ask about first.".to_string(),
            action_items: vec![ai("Reorder September investor update to lead with retention over signups", Some("Pushkar"), None, Some(starts[2]))],
            decisions: vec![dec("Lead the September update with retention numbers rather than signup counts", Some("I'd lead with retention rather than signups though"), None, Some(starts[1]))],
            open_questions: vec![],
        }).await?;
        series_a_last = series_a_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Good call overall. They want to see one more month of data before the partner meeting."),
            ("system", "That tracks with what we heard from the associate too. Worth sending it proactively rather than waiting to be asked."),
            ("mic", "Agreed, I'll set a reminder for the first week of next month."),
        ];
        let (_, starts) = build_turns(&lines, 360_000);
        let ended = seed_conv(&storage, &series_a.id, now, ConvSpec {
            title: "Follow-up — Sequoia partner call",
            days_ago: 20,
            hour_offset_s: 4500,
            duration_s: 360,
            lines,
            summary: "Sequoia wants one more month of data before the partner meeting, consistent with earlier signal from the associate. Plan is to send it proactively rather than wait to be asked.".to_string(),
            action_items: vec![ai("Send Sequoia the additional month of data proactively, first week of next month", Some("Pushkar"), Some("first week of next month"), Some(starts[2]))],
            decisions: vec![],
            open_questions: vec![oq("Whether Sequoia's partner meeting timeline slips if the extra data doesn't move the retention number", None, Some(starts[0]))],
        }).await?;
        series_a_last = series_a_last.max(ended);
    }

    storage.write_project_memory(&series_a.id, &serde_json::json!({
        "overview_markdown": "Series A is the current fundraise. Redpoint and Sequoia are both mid-diligence, with the metrics packet's month-three retention dip (attributed to the March pricing-change cohort) as the main question either side has raised. References came back positive with one minor support-responsiveness note now folded into the investor FAQ proactively.\n\nTerm sheet review turned up two items worth negotiating: the information rights schedule is broader than typical for this stage and is being redlined, while the board observer seat language needs an explicit non-voting confirmation. Pro-rata rights were accepted as drafted rather than contested.\n\nThe September investor update is in progress, reframed to lead with retention over signups per board feedback. Sequoia has asked for one additional month of data before their partner meeting; plan is to send it proactively in early next month rather than wait.",
        "scope_drift_markdown": "",
        "supersessions": [],
        "last_refresh_at": series_a_last,
        "last_refresh_runner": "claude",
    })).await?;

    // ---------------------------------------------------------------
    // Project 2: Aurora Launch
    // ---------------------------------------------------------------
    let aurora = storage
        .create_project(NewProject {
            name: "Aurora Launch".to_string(),
            description: Some("Planning and execution for the Aurora product launch.".to_string()),
        })
        .await?;

    let mut aurora_last = 0i64;

    // Rich #1: Kickoff sync
    {
        let lines = vec![
            ("mic", "Let's start with scope. Aurora is the workspace-sync feature — real-time sync across devices, no manual export. I want us aligned on what's in v1 and what waits."),
            ("system", "From the eng side, real-time sync across two devices is achievable by the target date. Three or more devices concurrently is where it gets risky — conflict resolution isn't solved yet."),
            ("mic", "Let's scope v1 to two devices, then. Three-plus can be a fast-follow if the architecture supports it without a rewrite."),
            ("system", "It does — we're building the conflict log so any device count could theoretically join once we've tested it. It's a testing and QA time question, not an architecture one."),
            ("mic", "Good, that's the kind of constraint I wanted to hear before we set the launch date."),
            ("system", "On timeline — if scope stays at two devices, we can be feature-complete in five weeks, then two weeks of hardening."),
            ("mic", "So a seven-week runway from today puts us at a launch date in mid-October."),
            ("system", "That's tight but workable if nothing major breaks in week five."),
            ("mic", "Let's set the target date as mid-October, with the explicit understanding that hardening isn't compressible if something surfaces."),
            ("system", "Agreed. I'd rather slip the date than ship broken sync."),
            ("mic", "On marketing — what needs to be true two weeks out?"),
            ("system", "Messaging draft, a landing page, and an email to the existing base. We also want a short walkthrough video, but that's the first thing I'd cut if we're tight on time."),
            ("mic", "Cut the video before the date. I'd rather launch on time with strong copy than late with a video."),
            ("system", "Understood. I'll have a messaging draft by next week for review."),
            ("mic", "On support — anything we need to prep before launch?"),
            ("system", "A troubleshooting doc for sync conflicts, and a way to tell from the dashboard which device last wrote a given file. Right now there's no visibility into that."),
            ("mic", "That visibility feature — is that in scope for v1 or does it wait?"),
            ("system", "It's small, mostly a UI addition on top of the conflict log Eng is already building. I think it fits."),
            ("mic", "Let's include it, then, since the underlying data already exists. I don't want support debugging sync issues blind."),
            ("system", "I'll add it to the eng ticket list this week."),
            ("mic", "Good. Let's recap: two-device sync in scope, three-plus deferred, mid-October target, video cut from launch assets, last-write-device visibility added to v1. Anything I'm missing?"),
            ("system", "Just a question — do we soft-launch to a subset of users first, or go to everyone at once?"),
            ("mic", "Open question, let's decide that closer to the date once we see how hardening goes."),
        ];
        let (_, starts) = build_turns(&lines, 1_800_000);
        let ended = seed_conv(&storage, &aurora.id, now, ConvSpec {
            title: "Kickoff sync",
            days_ago: 63,
            hour_offset_s: 3600,
            duration_s: 1800,
            lines,
            summary: "Kicked off Aurora, the real-time workspace-sync feature. V1 scope is locked to two-device sync — conflict resolution across three or more devices concurrently isn't proven yet, so that's deferred to a fast-follow once the conflict log has been tested more broadly. That log is architected to support any device count later without a rewrite, so the deferral is a testing-time constraint, not a technical one.\n\nTimeline: five weeks to feature-complete, two weeks of hardening, landing on a mid-October target. The team was explicit that hardening time is not compressible — if something surfaces in week five, the date moves rather than the hardening gets cut.\n\nMarketing's launch checklist is messaging, a landing page, and an email to the existing base; the walkthrough video was named upfront as the first thing to cut under time pressure, and the date was set as the thing that doesn't move instead. Support asked for a troubleshooting doc plus dashboard visibility into which device last wrote a given file — the latter got pulled into v1 scope since it rides on data the conflict log already produces.\n\nOpen question heading into hardening: whether to soft-launch to a subset of users first or go to everyone at once. Deferred until hardening results are in.".to_string(),
            action_items: vec![
                ai("Draft launch messaging for review", Some("Maya"), Some("next week"), Some(starts[13])),
                ai("Add last-write-device visibility to the dashboard as a v1 ticket", Some("Eng"), Some("this week"), Some(starts[19])),
                ai("Write a troubleshooting doc for sync conflicts", Some("Support"), Some("before launch"), Some(starts[15])),
                ai("Decide soft-launch vs. full launch once hardening results are in", Some("Pushkar"), Some("closer to launch date"), Some(starts[22])),
            ],
            decisions: vec![
                dec("Scope Aurora v1 to real-time sync across exactly two devices; three-or-more-device sync deferred to a fast-follow", Some("Let's scope v1 to two devices, then."), Some("Pushkar"), Some(starts[2])),
                dec("Set the launch target date as mid-October, with hardening time treated as non-negotiable", Some("I'd rather slip the date than ship broken sync."), Some("Eng"), Some(starts[9])),
                dec("Cut the launch walkthrough video before cutting the mid-October date", Some("Cut the video before the date. I'd rather launch on time with strong copy than late with a video."), Some("Pushkar"), Some(starts[12])),
            ],
            open_questions: vec![oq("Do we soft-launch to a subset of users first, or go to everyone at once?", Some("Eng"), Some(starts[21]))],
        }).await?;
        aurora_last = aurora_last.max(ended);
    }

    // Rich #2: Pricing review
    {
        let lines = vec![
            ("mic", "Let's land pricing today. Two options on the table — Aurora as a $12/month add-on, or bundled into the existing Pro tier at no extra charge."),
            ("system", "I'd push back on bundling it for free. Real-time sync is the single most requested feature in the last two quarters of feedback — that's willingness to pay we'd be leaving on the table."),
            ("mic", "That's fair, but bundling drives Pro upgrades from the free tier, which is our bigger lever right now. An add-on only sells to people already on Pro."),
            ("system", "It doesn't have to be either-or. What if it's bundled into Pro, but stays a paid add-on for the free tier? That gives us the upgrade lever and captures willingness to pay from people who won't upgrade to Pro anyway."),
            ("mic", "I like that shape better than either extreme. What price for the free-tier add-on?"),
            ("system", "Feedback panel said $12 felt fair, $20 felt steep. I'd go $12 to start — easy to raise later, harder to lower."),
            ("mic", "Agreed, raising is always easier than lowering. Let's set it at $12 for free-tier users, bundled at no charge for Pro."),
            ("system", "One risk — free-tier users might just upgrade to Pro instead of paying $12 standalone, since Pro is $15. That cannibalizes the add-on."),
            ("mic", "That's actually fine, isn't it? Either way we get the revenue, and a Pro upgrade is stickier than an add-on subscription."),
            ("system", "True, I hadn't thought of it that way. I'll stop worrying about that specific risk."),
            ("mic", "What about annual pricing — do we discount the add-on the same as we discount Pro annual?"),
            ("system", "I'd match the existing 20% annual discount rather than invent a new number. Consistency matters more than optimizing this one line item."),
            ("mic", "Agreed. Let's keep the same 20% annual discount structure."),
            ("system", "Last thing — grandfathering. Anyone already on a beta of Aurora during the test period, do they keep it free?"),
            ("mic", "Yes. Anyone in the beta cohort keeps free access permanently. That was implied when we invited them and I don't want to walk it back."),
            ("system", "I'll get that list from the beta program and flag it to billing so it's excluded from the paywall at launch."),
            ("mic", "Good. I think we have a full pricing model now — bundled in Pro, $12/month standalone for free tier, 20% annual discount, beta cohort grandfathered."),
            ("system", "I'll write it up and send it to finance for the revenue model update."),
        ];
        let (_, starts) = build_turns(&lines, 1_680_000);
        let ended = seed_conv(&storage, &aurora.id, now, ConvSpec {
            title: "Pricing review",
            days_ago: 40,
            hour_offset_s: 5400,
            duration_s: 1680,
            lines,
            summary: "Worked through Aurora pricing, starting from two extremes — a standalone $12/month add-on, or free bundling into Pro — and landed on a hybrid: bundled at no extra cost for Pro-tier users, and a $12/month standalone add-on for the free tier. The $12 figure came from feedback-panel testing, where $20 read as steep; the reasoning for starting low was explicit — it's easier to raise a price later than to lower one.\n\nA cannibalization concern came up — free-tier users might upgrade to Pro ($15) rather than pay $12 for the add-on — but the conclusion was that this is fine either way, since a Pro upgrade is stickier revenue than a standalone add-on subscription. Annual pricing will match the existing 20% discount rather than introduce a new number, on the reasoning that consistency across the pricing structure matters more than optimizing this one line.\n\nBeta cohort users keep free access permanently, since that was the implicit deal when they were invited into the test period. That list needs to reach billing before launch so those accounts are excluded from the new paywall. Full model — bundled Pro, $12/month free-tier add-on, 20% annual discount, beta grandfathering — is being written up for finance's revenue model update.".to_string(),
            action_items: vec![
                ai("Pull the beta cohort list and flag it to billing as grandfathered/excluded from the paywall", Some("Pushkar"), Some("before launch"), Some(starts[15])),
                ai("Write up the final Aurora pricing model and send to finance for the revenue model update", Some("Maya"), Some("this week"), Some(starts[17])),
                ai("Confirm Pro-tier annual pricing math accounts for bundled Aurora before the model goes to finance", Some("Pushkar"), Some("before launch"), Some(starts[12])),
            ],
            decisions: vec![
                dec("Aurora ships bundled at no extra cost for Pro-tier users, and as a $12/month standalone add-on for free-tier users", Some("I like that shape better than either extreme."), Some("Pushkar"), Some(starts[4])),
                dec("Set the free-tier add-on price at $12/month rather than $20", Some("I'd go $12 to start — easy to raise later, harder to lower."), Some("Pushkar"), Some(starts[6])),
                dec("Apply the existing 20% annual discount to the Aurora add-on instead of introducing a new discount tier", Some("Consistency matters more than optimizing this one line item."), Some("Pushkar"), Some(starts[12])),
            ],
            open_questions: vec![oq("Whether free-tier users who add Aurora standalone should get a prompted nudge to upgrade to Pro instead", Some("Maya"), Some(starts[9]))],
        }).await?;
        aurora_last = aurora_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Where are we on the conflict log?"),
            (
                "system",
                "Merged yesterday. Two-device sync is passing in staging now.",
            ),
            ("mic", "Good, that unblocks hardening week on schedule."),
        ];
        let (_, starts) = build_turns(&lines, 240_000);
        let ended = seed_conv(&storage, &aurora.id, now, ConvSpec {
            title: "Eng standup",
            days_ago: 25,
            hour_offset_s: 1800,
            duration_s: 240,
            lines,
            summary: "Conflict log merged and two-device sync is passing in staging, which keeps hardening week on schedule.".to_string(),
            action_items: vec![ai("Kick off hardening week testing once staging soak completes", Some("Eng"), Some("this week"), Some(starts[1]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        aurora_last = aurora_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "How's the landing page looking?"),
            ("system", "First draft is up for review. I trimmed the video reference since we cut that from launch assets."),
            ("mic", "Good, send me the link and I'll review today."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &aurora.id, now, ConvSpec {
            title: "Marketing sync",
            days_ago: 15,
            hour_offset_s: 3600,
            duration_s: 300,
            lines,
            summary: "Landing page first draft is ready for review, updated to drop the walkthrough-video reference per the earlier scope decision.".to_string(),
            action_items: vec![ai("Review the Aurora landing page draft", Some("Pushkar"), Some("today"), Some(starts[1]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        aurora_last = aurora_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Running through the checklist — sync hardening, support doc, landing page, billing changes. Anything red?"),
            ("system", "Billing is the one open item — the grandfathered beta list hasn't been loaded yet."),
            ("mic", "I'll chase that today, it shouldn't hold the date."),
            ("system", "Everything else is green."),
        ];
        let (_, starts) = build_turns(&lines, 480_000);
        let ended = seed_conv(&storage, &aurora.id, now, ConvSpec {
            title: "Launch readiness check",
            days_ago: 7,
            hour_offset_s: 2700,
            duration_s: 480,
            lines,
            summary: "Launch checklist is green across sync hardening, support docs, and the landing page. The only open item is loading the grandfathered beta cohort into billing, which isn't expected to hold the date.".to_string(),
            action_items: vec![ai("Load the grandfathered beta cohort list into billing before launch", Some("Pushkar"), Some("today"), Some(starts[1]))],
            decisions: vec![dec("Proceed with the mid-October launch date; only the billing list load remains open", None, Some("Pushkar"), Some(starts[2]))],
            open_questions: vec![],
        }).await?;
        aurora_last = aurora_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Overall how do we think it went?"),
            ("system", "Smooth technically — no major sync incidents in the first 48 hours. Signups on the landing page were softer than we hoped though."),
            ("mic", "Any theory on why?"),
            ("system", "I think cutting the walkthrough video cost us more than we expected. A few people in support tickets mentioned wanting to see it work before trying it."),
            ("mic", "Worth revisiting for the next launch — maybe the video isn't the first thing to cut next time."),
            ("system", "Agreed, I'll note that for the launch playbook."),
        ];
        let (_, starts) = build_turns(&lines, 600_000);
        let ended = seed_conv(&storage, &aurora.id, now, ConvSpec {
            title: "Launch retro",
            days_ago: 1,
            hour_offset_s: 3600,
            duration_s: 600,
            lines,
            summary: "Aurora launched cleanly on the technical side — no major sync incidents in the first 48 hours. Landing page signups came in softer than hoped, with a working theory that cutting the walkthrough video (decided at kickoff, under time pressure) cost more than expected; several support tickets mentioned wanting to see sync working before trying it.".to_string(),
            action_items: vec![ai("Update the launch playbook to reconsider cutting walkthrough videos under time pressure", Some("Maya"), Some("before next launch"), Some(starts[5]))],
            decisions: vec![],
            open_questions: vec![oq("Whether a post-launch video now would still meaningfully lift signups", Some("Pushkar"), Some(starts[3]))],
        }).await?;
        aurora_last = aurora_last.max(ended);
    }

    storage.write_project_memory(&aurora.id, &serde_json::json!({
        "overview_markdown": "Aurora is the real-time workspace-sync feature. V1 scope was locked at kickoff to two-device sync, with three-or-more-device support deferred to a fast-follow once the conflict log — already architected to support any device count — has had more testing time. Target launch date was set at mid-October, with the team explicit that hardening time would not be compressed to protect the date.\n\nPricing landed on a hybrid model after real debate: bundled free for Pro-tier users, a $12/month standalone add-on for the free tier. The lower price point was a deliberate choice (easier to raise than lower), annual pricing matches the existing 20% Pro discount for consistency, and the beta test cohort keeps permanent free access.\n\nLaunch shipped on schedule and technically clean — no major sync incidents in the first 48 hours. Signups came in softer than hoped, and the retro's working theory is that cutting the walkthrough video (a kickoff decision made under timeline pressure) cost more than anticipated; that's now flagged for the launch playbook.",
        "scope_drift_markdown": "One net scope addition since kickoff: last-write-device visibility on the dashboard was pulled into v1 (originally discussed as a maybe) because it rode on data the conflict log already produces. No other scope changes — the two-device boundary and mid-October date both held through launch.",
        "supersessions": [],
        "last_refresh_at": aurora_last,
        "last_refresh_runner": "claude",
    })).await?;

    // ---------------------------------------------------------------
    // Project 3: Hiring — Platform Team
    // ---------------------------------------------------------------
    let hiring = storage
        .create_project(NewProject {
            name: "Hiring — Platform Team".to_string(),
            description: Some(
                "Interview loops and hiring decisions for open roles on the platform team."
                    .to_string(),
            ),
        })
        .await?;

    let mut hiring_last = 0i64;

    {
        let lines = vec![
            ("mic", "Walk me through your experience with distributed systems."),
            ("system", "Mostly at my last role — built the sharding layer for a multi-tenant queue system, about three years on it."),
            ("mic", "What was the hardest part of that?"),
            ("system", "Rebalancing shards without downtime. We ended up building a gradual migration path rather than a hard cutover."),
        ];
        let (_, starts) = build_turns(&lines, 360_000);
        let ended = seed_conv(&storage, &hiring.id, now, ConvSpec {
            title: "Screen — J. Alvarez",
            days_ago: 35,
            hour_offset_s: 3600,
            duration_s: 360,
            lines,
            summary: "Strong initial screen. Three years building a sharding layer for a multi-tenant queue system, with a specific, credible answer on the hardest part (zero-downtime shard rebalancing via gradual migration rather than a hard cutover).".to_string(),
            action_items: vec![ai("Move J. Alvarez to the panel round", Some("Pushkar"), Some("this week"), Some(starts[3]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        hiring_last = hiring_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Panel feedback on Alvarez?"),
            ("system", "Strong on systems design, a bit light on hands-on Rust — he's coming from Go. Everyone leaned positive though."),
            ("mic", "Ramp risk on Rust — how big a concern is that really?"),
            ("system", "Not huge. Go and Rust aren't that far apart for someone who's already thinking in those patterns. I'd bet on him."),
        ];
        let (_, starts) = build_turns(&lines, 420_000);
        let ended = seed_conv(&storage, &hiring.id, now, ConvSpec {
            title: "Panel debrief — backend role",
            days_ago: 28,
            hour_offset_s: 4500,
            duration_s: 420,
            lines,
            summary: "Panel leaned positive on Alvarez overall. Systems design was the strongest signal; hands-on Rust experience is thinner since his background is Go, but the panel judged the ramp risk to be low.".to_string(),
            action_items: vec![ai("Draft an offer for J. Alvarez", Some("Pushkar"), Some("this week"), Some(starts[3]))],
            decisions: vec![dec("Move J. Alvarez to offer stage despite limited direct Rust experience", Some("I'd bet on him."), Some("panel"), Some(starts[3]))],
            open_questions: vec![],
        }).await?;
        hiring_last = hiring_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "What's the comp band looking like?"),
            ("system", "Mid-point of the senior band, plus the standard equity grant. He mentioned a competing offer so we may need to move on timeline."),
            ("mic", "Let's get the offer out by Friday rather than sit on it."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &hiring.id, now, ConvSpec {
            title: "Offer discussion",
            days_ago: 21,
            hour_offset_s: 2700,
            duration_s: 300,
            lines,
            summary: "Comp set at the mid-point of the senior band plus the standard equity grant. Alvarez has a competing offer, so timeline matters — plan is to send by Friday rather than let it sit.".to_string(),
            action_items: vec![ai("Send J. Alvarez's offer by Friday", Some("Pushkar"), Some("Friday"), Some(starts[2]))],
            decisions: vec![],
            open_questions: vec![oq("Whether the competing offer changes the comp band we should open with", None, Some(starts[1]))],
        }).await?;
        hiring_last = hiring_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Tell me about the caching project you mentioned."),
            ("system", "Built a write-through cache layer in front of our primary datastore, cut p99 latency by about 40 percent."),
            ("mic", "What tradeoffs did you run into?"),
            ("system", "Cache invalidation on writes was the tricky part — we ended up with a short TTL rather than trying to invalidate perfectly."),
        ];
        let (_, starts) = build_turns(&lines, 360_000);
        let ended = seed_conv(&storage, &hiring.id, now, ConvSpec {
            title: "Screen — M. Chen",
            days_ago: 14,
            hour_offset_s: 3600,
            duration_s: 360,
            lines,
            summary: "Solid screen. Built a write-through cache layer that cut p99 latency roughly 40 percent, with a clear-eyed answer on the invalidation tradeoff (short TTL over exact invalidation).".to_string(),
            action_items: vec![ai("Move M. Chen to the panel round", Some("Pushkar"), Some("next week"), Some(starts[3]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        hiring_last = hiring_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "How'd the panel go?"),
            ("system", "Good signal across the board, especially on the systems design round. One panelist flagged so-so communication in the pairing exercise."),
            ("mic", "Worth a follow-up call, or is that not disqualifying?"),
            ("system", "Not disqualifying. I think it was nerves — the design round was much sharper."),
        ];
        let (_, starts) = build_turns(&lines, 360_000);
        let ended = seed_conv(&storage, &hiring.id, now, ConvSpec {
            title: "Panel debrief — M. Chen",
            days_ago: 6,
            hour_offset_s: 4500,
            duration_s: 360,
            lines,
            summary: "Panel signal was good overall, strongest in the systems design round. One panelist noted so-so communication during pairing, attributed to nerves rather than a real gap given how much sharper the design round was.".to_string(),
            action_items: vec![],
            decisions: vec![dec("Proceed to offer for M. Chen; communication concern from the pairing round noted but not disqualifying", Some("Not disqualifying."), Some("panel"), Some(starts[3]))],
            open_questions: vec![oq("Whether to loop in a second communication-focused interview before the offer, or proceed directly", None, Some(starts[2]))],
        }).await?;
        hiring_last = hiring_last.max(ended);
    }

    storage.write_project_memory(&hiring.id, &serde_json::json!({
        "overview_markdown": "Hiring for the platform team, currently running two candidates through the loop. J. Alvarez screened and panel'd well — strong systems design, thinner on hands-on Rust since his background is Go, but the panel judged ramp risk low and moved him to offer. He has a competing offer, so the plan is to send by Friday rather than let timeline slip.\n\nM. Chen is a step behind Alvarez in the process — screened well on caching and cache-invalidation tradeoffs, then panel'd with good signal, particularly on systems design. One panelist flagged so-so communication during the pairing exercise, attributed to nerves rather than a real signal; the panel's read is to proceed to offer, with an open question on whether a second communication-focused interview is worth adding first.",
        "scope_drift_markdown": "",
        "supersessions": [],
        "last_refresh_at": hiring_last,
        "last_refresh_runner": "claude",
    })).await?;

    // ---------------------------------------------------------------
    // Project 4: 1:1s — David (weekly)
    // ---------------------------------------------------------------
    let david = storage
        .create_project(NewProject {
            name: "1:1s — David".to_string(),
            description: Some("Weekly 1:1s with David.".to_string()),
        })
        .await?;

    let mut david_last = 0i64;

    {
        let lines = vec![
            ("mic", "How's the sprint going?"),
            ("system", "On track. The migration script took longer than expected but it's done now."),
            ("mic", "Anything blocking you this week?"),
            ("system", "Nothing major, just want another pair of eyes on the rollback plan before I ship it."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &david.id, now, ConvSpec {
            title: "David — Jul 25",
            days_ago: 37,
            hour_offset_s: 1800,
            duration_s: 300,
            lines,
            summary: "Sprint on track; migration script took longer than planned but is done. David wants a second set of eyes on the rollback plan before shipping.".to_string(),
            action_items: vec![ai("Review David's rollback plan before he ships the migration", Some("Pushkar"), Some("this week"), Some(starts[3]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        david_last = david_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "How are you feeling about the workload lately?"),
            ("system", "Honestly a bit stretched with the on-call rotation on top of the feature work. Manageable, but I wanted to flag it."),
            ("mic", "Appreciate you saying so. Let's look at rebalancing on-call next planning cycle."),
        ];
        let (_, starts) = build_turns(&lines, 360_000);
        let ended = seed_conv(&storage, &david.id, now, ConvSpec {
            title: "David — Aug 1",
            days_ago: 30,
            hour_offset_s: 2700,
            duration_s: 360,
            lines,
            summary: "David flagged feeling stretched between the on-call rotation and feature work — manageable for now, but worth addressing at the next planning cycle.".to_string(),
            action_items: vec![ai("Look at rebalancing David's on-call load next planning cycle", Some("Pushkar"), Some("next planning cycle"), Some(starts[2]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        david_last = david_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Any update on the on-call conversation?"),
            ("system", "Better already, actually — swapped one rotation with Sam. Feature work is moving well too."),
            ("mic", "Good to hear. Anything you want to dig into for growth this quarter?"),
            ("system", "I've been meaning to get more reps leading design reviews. Could use a nudge to actually schedule one."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &david.id, now, ConvSpec {
            title: "David — Aug 8",
            days_ago: 23,
            hour_offset_s: 3600,
            duration_s: 300,
            lines,
            summary: "On-call already better after a rotation swap with Sam. David wants more reps leading design reviews this quarter and asked for a nudge to actually schedule one.".to_string(),
            action_items: vec![ai("Find David an opportunity to lead a design review this quarter", Some("Pushkar"), Some("this quarter"), Some(starts[3]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        david_last = david_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "How'd the design review go?"),
            (
                "system",
                "Ran it Tuesday. Went well, though I under-prepped the alternatives section a bit.",
            ),
            (
                "mic",
                "That's normal for a first one. Want to do another before the quarter's out?",
            ),
            ("system", "Yes, I'll find a good candidate project."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &david.id, now, ConvSpec {
            title: "David — Aug 15",
            days_ago: 16,
            hour_offset_s: 1800,
            duration_s: 300,
            lines,
            summary: "David's first design review went well, with light under-prep on the alternatives section — normal for a first run. He wants to lead a second one before quarter end.".to_string(),
            action_items: vec![ai("David to pick a second design review candidate before quarter end", Some("David"), Some("before quarter end"), Some(starts[3]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        david_last = david_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Quick check-in — anything on your mind this week?"),
            (
                "system",
                "Not really, pretty quiet week. Good chance to catch up on backlog.",
            ),
            ("mic", "Enjoy the quiet one while it lasts."),
        ];
        let (_, _starts) = build_turns(&lines, 240_000);
        let ended = seed_conv(
            &storage,
            &david.id,
            now,
            ConvSpec {
                title: "David — Aug 22",
                days_ago: 9,
                hour_offset_s: 2700,
                duration_s: 240,
                lines,
                summary:
                    "Quiet week, mostly spent catching up on backlog. Nothing pressing to flag."
                        .to_string(),
                action_items: vec![],
                decisions: vec![],
                open_questions: vec![],
            },
        )
        .await?;
        david_last = david_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "How's the new on-call split working out?"),
            ("system", "Much better, no complaints. I also finished the second design review prep — scheduled for next week."),
            ("mic", "Nice, that's good momentum."),
        ];
        let (_, starts) = build_turns(&lines, 240_000);
        let ended = seed_conv(&storage, &david.id, now, ConvSpec {
            title: "David — Aug 29",
            days_ago: 2,
            hour_offset_s: 3600,
            duration_s: 240,
            lines,
            summary: "On-call rebalancing is working well with no complaints. Second design review is prepped and scheduled for next week.".to_string(),
            action_items: vec![],
            decisions: vec![],
            open_questions: vec![oq("Whether the on-call rebalancing should become the permanent rotation or was just a one-off swap", Some("Pushkar"), Some(starts[0]))],
        }).await?;
        david_last = david_last.max(ended);
    }

    storage.write_project_memory(&david.id, &serde_json::json!({
        "overview_markdown": "Weekly 1:1s with David. Main throughline over the past two months: an on-call rotation that was crowding out feature work got flagged early, addressed with a rotation swap, and has been running smoothly since — worth revisiting whether that swap should become the permanent rotation rather than staying informal.\n\nOn the growth side, David asked for more reps leading design reviews. He's run two since then, the first with some under-prep on the alternatives section (normal for a first pass), the second fully prepped and scheduled. Otherwise a fairly steady stretch — one quiet week with nothing to report, one migration ship that went cleanly after a rollback-plan review.",
        "scope_drift_markdown": "",
        "supersessions": [],
        "last_refresh_at": david_last,
        "last_refresh_runner": "claude",
    })).await?;

    // ---------------------------------------------------------------
    // Project 5: 1:1s — Priya (biweekly)
    // ---------------------------------------------------------------
    let priya = storage
        .create_project(NewProject {
            name: "1:1s — Priya".to_string(),
            description: Some("Biweekly 1:1s with Priya.".to_string()),
        })
        .await?;

    let mut priya_last = 0i64;

    {
        let lines = vec![
            ("mic", "How's the roadmap planning coming along?"),
            ("system", "Good progress, though prioritization between the Aurora dependencies and the platform migration is getting tight."),
            ("mic", "Let's timebox that discussion for next week's planning rather than solve it here."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &priya.id, now, ConvSpec {
            title: "Priya — Jun 19",
            days_ago: 73,
            hour_offset_s: 3600,
            duration_s: 300,
            lines,
            summary: "Roadmap planning progressing, but prioritization between Aurora dependencies and the platform migration is getting tight. Deferred to next week's planning session rather than resolved ad hoc.".to_string(),
            action_items: vec![ai("Timebox Aurora-vs-platform-migration prioritization for next week's planning", Some("Pushkar"), Some("next week"), Some(starts[2]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        priya_last = priya_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Did the prioritization conversation land somewhere workable?"),
            ("system", "Yes, platform migration slips two weeks, Aurora stays on track. Team seemed fine with it."),
            ("mic", "Good, that's the right tradeoff for now."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &priya.id, now, ConvSpec {
            title: "Priya — Jul 3",
            days_ago: 59,
            hour_offset_s: 2700,
            duration_s: 300,
            lines,
            summary: "Prioritization resolved: platform migration slips two weeks so Aurora stays on track. Team took it in stride.".to_string(),
            action_items: vec![],
            decisions: vec![dec("Slip the platform migration by two weeks to keep Aurora on track", Some("Team seemed fine with it."), Some("Priya"), Some(starts[1]))],
            open_questions: vec![],
        }).await?;
        priya_last = priya_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "How's morale on the team generally?"),
            ("system", "Pretty good. One person mentioned wanting more context on the longer-term roadmap, not just the current sprint."),
            ("mic", "Fair, I'll do a short roadmap walkthrough at the next team meeting."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &priya.id, now, ConvSpec {
            title: "Priya — Jul 17",
            days_ago: 45,
            hour_offset_s: 1800,
            duration_s: 300,
            lines,
            summary: "Morale generally good. One request came up for more visibility into the longer-term roadmap beyond the current sprint.".to_string(),
            action_items: vec![ai("Give the team a longer-term roadmap walkthrough at the next team meeting", Some("Pushkar"), Some("next team meeting"), Some(starts[2]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        priya_last = priya_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Did the roadmap walkthrough land well?"),
            ("system", "Yes, good questions afterward. People seem clearer on the why now, not just the what."),
            ("mic", "Glad it helped."),
        ];
        let (_, _starts) = build_turns(&lines, 240_000);
        let ended = seed_conv(&storage, &priya.id, now, ConvSpec {
            title: "Priya — Jul 31",
            days_ago: 31,
            hour_offset_s: 2700,
            duration_s: 240,
            lines,
            summary: "Roadmap walkthrough landed well — good follow-up questions, and the team seems clearer on the reasoning behind priorities, not just the priorities themselves.".to_string(),
            action_items: vec![],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        priya_last = priya_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "How's the platform migration going now that it's back on track?"),
            ("system", "On track, actually ahead by a couple of days. I want to use the buffer for a bit of tech debt cleanup rather than pulling the date in."),
            ("mic", "Agreed, spend the buffer on debt rather than compressing the timeline."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &priya.id, now, ConvSpec {
            title: "Priya — Aug 14",
            days_ago: 17,
            hour_offset_s: 3600,
            duration_s: 300,
            lines,
            summary: "Platform migration is running a couple of days ahead. Rather than pulling the finish date in, the plan is to spend the schedule buffer on tech debt cleanup.".to_string(),
            action_items: vec![],
            decisions: vec![dec("Use the platform migration's schedule buffer for tech debt cleanup instead of pulling the finish date in", Some("Agreed, spend the buffer on debt rather than compressing the timeline."), Some("Pushkar"), Some(starts[2]))],
            open_questions: vec![],
        }).await?;
        priya_last = priya_last.max(ended);
    }

    {
        let lines = vec![
            ("mic", "Anything on your radar heading into September?"),
            ("system", "Just headcount planning for next quarter — want to start early this time instead of scrambling in December."),
            ("mic", "Good instinct. Let's put together a draft together next week."),
        ];
        let (_, starts) = build_turns(&lines, 300_000);
        let ended = seed_conv(&storage, &priya.id, now, ConvSpec {
            title: "Priya — Aug 28",
            days_ago: 3,
            hour_offset_s: 1800,
            duration_s: 300,
            lines,
            summary: "Priya wants to start Q4 headcount planning early rather than scrambling in December. Plan is to draft it together next week.".to_string(),
            action_items: vec![ai("Draft Q4 headcount plan with Priya", Some("Pushkar"), Some("next week"), Some(starts[2]))],
            decisions: vec![],
            open_questions: vec![],
        }).await?;
        priya_last = priya_last.max(ended);
    }

    storage.write_project_memory(&priya.id, &serde_json::json!({
        "overview_markdown": "Biweekly 1:1s with Priya. Main thread over the summer was a prioritization crunch between Aurora dependencies and the platform migration — resolved by slipping the migration two weeks, which the team absorbed without much friction. The migration has since pulled ahead of its revised schedule by a couple of days; rather than pulling the finish date in, the plan is to spend that buffer on tech debt cleanup.\n\nA roadmap-visibility request from the team led to a walkthrough at a team meeting, which landed well — people came away clearer on the reasoning behind priorities, not just the priority list itself. Heading into September, Priya wants to start Q4 headcount planning early rather than the usual December scramble.",
        "scope_drift_markdown": "",
        "supersessions": [],
        "last_refresh_at": priya_last,
        "last_refresh_runner": "claude",
    })).await?;

    // ---------------------------------------------------------------
    eprintln!("done: 5 projects seeded");
    eprintln!("  Series A: 6 conversations, last activity at {series_a_last}");
    eprintln!("  Aurora Launch: 6 conversations (rich: Kickoff sync, Pricing review), last activity at {aurora_last}");
    eprintln!("  Hiring — Platform Team: 5 conversations, last activity at {hiring_last}");
    eprintln!("  1:1s — David: 6 conversations, last activity at {david_last}");
    eprintln!("  1:1s — Priya: 6 conversations, last activity at {priya_last}");

    Ok(())
}
