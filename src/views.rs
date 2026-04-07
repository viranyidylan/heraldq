use crate::db::stats::ActivityCounts;

pub fn render_dashboard(counts: &ActivityCounts) -> String {
    let received = counts.delivered_today + counts.failed_today + counts.queued;

    format!(
        r#"<!DOCTYPE html>
<html><head><title>sliq dashboard</title>
<style>body{{font-family:monospace;background:#111;color:#ccc;padding:2rem}}
h1{{color:#fff;font-size:1.2rem}}pre{{font-size:1rem;line-height:1.8}}</style>
</head><body>
<h1>webhook activity (today)</h1>
<pre>
received     {received}
queued       {queued}
delivered    {delivered}
failed       {failed}
dlq total    {dlq}
</pre>
</body></html>"#,
        received = received,
        queued = counts.queued,
        delivered = counts.delivered_today,
        failed = counts.failed_today,
        dlq = counts.dlq_total,
    )
}
