//! 流量统计 HTML 页面，浏览器访问 /sub/<token> 默认返回这个。
//!
//! 深色主题 + 进度条（按用量百分比分段配色）+ QR 码（SVG 内联）+ 复制按钮。

use qrcode::{render::svg, QrCode};
use serde_json::Value;

use crate::model::user::User;
use crate::service::sub_service;

/// 渲染完整 HTML。`base_url` 形如 `https://sub.example.com`（不带尾斜杠）。
pub fn render(cfg: &Value, user: &User, server: &str, base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let links = sub_service::generate_links(cfg, &user.name, server).unwrap_or_default();

    let used_total = user.used_total_bytes().max(0);
    let quota_bytes = user.quota_bytes();
    let pct = user.quota_used_percent();
    let expired = user.is_expired();
    let over = user.is_over_quota();

    let (bar_cls, status_cls, status_label) = if !user.enabled {
        ("bad", "bad", "已停用")
    } else if expired {
        ("bad", "bad", "已到期")
    } else if over {
        ("bad", "bad", "已超额")
    } else if pct >= 95.0 {
        ("bad", "bad", "即将耗尽")
    } else if pct >= 80.0 {
        ("warn", "warn", "偏高")
    } else {
        ("", "", "正常")
    };

    let total_str = if quota_bytes <= 0 { "不限".into() } else { User::format_bytes(quota_bytes) };
    let used_str  = User::format_bytes(used_total);
    let up_str    = User::format_bytes(user.used_up_bytes.max(0));
    let down_str  = User::format_bytes(user.used_down_bytes.max(0));

    let reset_desc = match user.reset_day {
        0  => "不重置".into(),
        32 => "每月 1 号".into(),
        d  => format!("每月 {} 号", d),
    };
    let expire_desc = describe_expire(&user.expire_at);

    let sub_sing = format!("{}/sub/{}", base, user.sub_token);
    let sub_clash = format!("{}/sub/{}?type=clash", base, user.sub_token);

    let sub_rows = format!(
        r#"<div class="row">
  <span class="name">sing-box / v2rayN</span>
  <code>{sing_h}</code>
  <button onclick="copy(this,'{sing_j}')">复制</button>
</div>
<div class="row">
  <span class="name">mihomo / Clash Meta</span>
  <code>{clash_h}</code>
  <button onclick="copy(this,'{clash_j}')">复制</button>
</div>"#,
        sing_h = html_escape(&sub_sing), sing_j = js_escape(&sub_sing),
        clash_h = html_escape(&sub_clash), clash_j = js_escape(&sub_clash),
    );

    let node_rows = if links.is_empty() {
        r#"<div style="color:var(--muted);font-size:13px;">暂无可用节点。先在 TUI 节点页添加，并给本用户分配节点。</div>"#.to_string()
    } else {
        links.iter().map(|l| {
            let qr = qrcode_svg(&l.link);
            format!(
                r#"<div class="node">
  <div class="row">
    <span class="name">{tag_h} <span style="color:var(--muted);">· {proto_h}</span></span>
    <code>{link_h}</code>
    <button onclick="copy(this,'{link_j}')">复制</button>
  </div>
  <details><summary>QR</summary>{qr}</details>
</div>"#,
                tag_h = html_escape(&l.tag), proto_h = html_escape(&l.protocol),
                link_h = html_escape(&l.link), link_j = js_escape(&l.link),
                qr = qr,
            )
        }).collect::<Vec<_>>().join("\n")
    };

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{name_h} · 订阅</title>
<style>
:root {{
  --bg:#0d1117; --card:#161b22; --border:#30363d;
  --text:#e6edf3; --muted:#8b949e; --accent:#58a6ff;
  --ok:#3fb950; --warn:#d29922; --bad:#f85149;
}}
* {{ box-sizing:border-box; }}
html,body {{ margin:0; padding:0; }}
body {{
  background:var(--bg); color:var(--text);
  font-family:-apple-system,BlinkMacSystemFont,"Segoe UI","Noto Sans SC",sans-serif;
  min-height:100vh; padding:24px 16px;
}}
.wrap {{ max-width:760px; margin:0 auto; }}
.card {{
  background:var(--card); border:1px solid var(--border);
  border-radius:12px; padding:20px; margin-bottom:16px;
}}
h1,h2 {{ margin-top:0; font-weight:600; }}
h1 {{ font-size:22px; display:flex; align-items:center; gap:12px; flex-wrap:wrap; }}
h2 {{ font-size:12px; color:var(--muted); text-transform:uppercase; letter-spacing:.8px; margin-bottom:14px; }}
.status {{ font-size:12px; padding:3px 10px; border-radius:999px; background:#1f2937; color:var(--ok); font-weight:500; }}
.status.warn {{ color:var(--warn); background:#2b2310; }}
.status.bad {{ color:var(--bad); background:#2b1010; }}
.meta {{ display:grid; grid-template-columns:1fr 1fr; gap:6px 16px; color:var(--muted); font-size:13px; margin-top:10px; }}
.meta span b {{ color:var(--text); font-weight:500; }}
.bar {{ height:10px; background:#21262d; border-radius:6px; overflow:hidden; margin:14px 0 8px; }}
.bar>span {{ display:block; height:100%; background:var(--ok); transition:width .3s ease; }}
.bar>span.warn {{ background:var(--warn); }}
.bar>span.bad {{ background:var(--bad); }}
.usage {{ font-size:14px; color:var(--muted); display:flex; justify-content:space-between; }}
.usage b {{ color:var(--text); font-weight:600; }}
.row {{ display:flex; align-items:center; gap:8px; margin-bottom:10px; flex-wrap:wrap; }}
.row:last-child {{ margin-bottom:0; }}
.row .name {{ min-width:150px; color:var(--muted); font-size:13px; }}
.row code {{
  flex:1; min-width:0; background:#010409; color:var(--accent);
  padding:8px 10px; border-radius:6px; font-size:12px;
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  font-family:ui-monospace,"SF Mono","Consolas",monospace;
  border:1px solid var(--border);
}}
button {{
  background:#21262d; color:var(--text); border:1px solid var(--border);
  border-radius:6px; padding:7px 14px; font-size:12px; cursor:pointer;
  transition:background .1s;
}}
button:hover {{ background:#30363d; }}
button.done {{ background:#1f6feb; border-color:#388bfd; color:#fff; }}
.node {{ padding:10px 0; border-top:1px solid var(--border); }}
.node:first-child {{ border-top:0; padding-top:0; }}
details summary {{ cursor:pointer; color:var(--muted); font-size:12px; margin-top:4px; user-select:none; }}
details[open] summary {{ color:var(--accent); }}
details svg {{ display:block; margin:10px auto 4px; background:#fff; padding:10px; border-radius:8px; max-width:220px; width:100%; height:auto; }}
.foot {{ text-align:center; color:var(--muted); font-size:11px; margin-top:24px; padding-bottom:12px; }}
.foot a {{ color:var(--muted); text-decoration:none; border-bottom:1px dotted var(--muted); }}
@media (max-width:520px) {{
  .meta {{ grid-template-columns:1fr; }}
  .row .name {{ min-width:0; flex-basis:100%; }}
}}
</style>
</head>
<body>
<div class="wrap">
  <div class="card">
    <h1>{name_h} <span class="status {status_cls}">{status_label}</span></h1>
    <div class="bar"><span class="{bar_cls}" style="width:{pct:.1}%"></span></div>
    <div class="usage"><span>已用 <b>{used}</b></span><span><b>{total}</b></span></div>
    <div class="meta">
      <span>重置: <b>{reset}</b></span>
      <span>到期: <b>{expire}</b></span>
      <span>上行: <b>{up}</b></span>
      <span>下行: <b>{down}</b></span>
    </div>
  </div>
  <div class="card">
    <h2>订阅导入</h2>
    {sub_rows}
  </div>
  <div class="card">
    <h2>单节点 ({n_nodes})</h2>
    {node_rows}
  </div>
  <div class="foot">由 sb-manager 生成 · <a href="https://github.com/why1f/singbox-manager" target="_blank">GitHub</a></div>
</div>
<script>
function copy(btn,text){{
  navigator.clipboard.writeText(text).then(function(){{
    var old=btn.textContent;
    btn.textContent='已复制';
    btn.classList.add('done');
    setTimeout(function(){{ btn.textContent=old; btn.classList.remove('done'); }},1200);
  }}).catch(function(){{ btn.textContent='复制失败'; }});
}}
</script>
</body>
</html>"#,
        name_h = html_escape(&user.name),
        status_cls = status_cls, status_label = status_label, bar_cls = bar_cls,
        pct = pct.min(100.0),
        used = used_str, total = total_str,
        reset = reset_desc, expire = expire_desc,
        up = up_str, down = down_str,
        sub_rows = sub_rows, node_rows = node_rows,
        n_nodes = links.len(),
    )
}

fn describe_expire(expire_at: &str) -> String {
    if expire_at.is_empty() { return "无限期".into(); }
    match chrono::NaiveDate::parse_from_str(expire_at, "%Y-%m-%d") {
        Ok(exp) => {
            let today = chrono::Local::now().date_naive();
            let days  = (exp - today).num_days();
            if days < 0   { format!("{} (已过期 {} 天)", exp, -days) }
            else if days == 0 { format!("{} (今日到期)", exp) }
            else              { format!("{} (还有 {} 天)", exp, days) }
        }
        Err(_) => expire_at.to_string(),
    }
}

fn qrcode_svg(data: &str) -> String {
    match QrCode::new(data.as_bytes()) {
        Ok(code) => code.render::<svg::Color>()
            .min_dimensions(200, 200)
            .dark_color(svg::Color("#0d1117"))
            .light_color(svg::Color("#ffffff"))
            .build(),
        Err(_) => "<div style='color:var(--muted);font-size:12px;'>QR 生成失败</div>".into(),
    }
}

/// HTML 文本节点/属性最小转义（属性我们只用双引号，所以也要转 " 和 '）
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<'  => out.push_str("&lt;"),
            '>'  => out.push_str("&gt;"),
            '&'  => out.push_str("&amp;"),
            '"'  => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c    => out.push(c),
        }
    }
    out
}

/// 在 JS 单引号字符串里安全嵌入
fn js_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '<'  => out.push_str("\\x3c"),  // 防止 </script> 截断
            c    => out.push(c),
        }
    }
    out
}
