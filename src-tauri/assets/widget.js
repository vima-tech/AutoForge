/* AutoForge embeddable feedback widget (M10).
 * Privacy: collects only feedback text + page URL. No identity, no cookies,
 * no device fingerprint. Screenshot capture is intentionally omitted.
 * Embed:
 *   <script src="http://HOST/widget.js"
 *           data-endpoint="http://HOST"
 *           data-project-id="PROJECT_ID"
 *           data-api-key="TOKEN"></script>
 */
(function () {
  var s = document.currentScript;
  if (!s) return;
  var endpoint = (s.getAttribute('data-endpoint') || '').replace(/\/$/, '');
  var project = s.getAttribute('data-project-id') || '';
  var token = s.getAttribute('data-api-key') || '';
  if (!endpoint || !project) return;

  var EMBER = '#e8772e';
  function el(tag, css, text) {
    var e = document.createElement(tag);
    if (css) e.style.cssText = css;
    if (text != null) e.textContent = text;
    return e;
  }

  var btn = el('button', 'position:fixed;right:20px;bottom:20px;z-index:99999;padding:10px 18px;border:none;border-radius:99px;background:' + EMBER + ';color:#fff;font:600 14px system-ui,sans-serif;cursor:pointer;box-shadow:0 6px 20px rgba(0,0,0,.25)', '反馈');
  document.body.appendChild(btn);

  var overlay = el('div', 'position:fixed;inset:0;z-index:100000;background:rgba(0,0,0,.45);display:none;align-items:center;justify-content:center');
  var card = el('div', 'width:360px;max-width:92vw;background:#fff;color:#222;border-radius:14px;overflow:hidden;font:14px system-ui,sans-serif;box-shadow:0 24px 60px rgba(0,0,0,.35)');
  var head = el('div', 'padding:14px 16px;font-weight:700;border-bottom:1px solid #eee', '提交反馈');
  var bodyWrap = el('div', 'padding:14px 16px;display:flex;flex-direction:column;gap:10px');
  var title = el('input', 'padding:9px 11px;border:1px solid #ddd;border-radius:8px;font:14px system-ui');
  title.placeholder = '一句话描述问题或建议';
  var desc = el('textarea', 'padding:9px 11px;border:1px solid #ddd;border-radius:8px;font:14px system-ui;min-height:80px;resize:vertical');
  desc.placeholder = '详细说明（可选）';
  var msg = el('div', 'font-size:12px;color:#888;min-height:16px');
  var actions = el('div', 'padding:12px 16px;display:flex;gap:8px;justify-content:flex-end;border-top:1px solid #eee');
  var cancel = el('button', 'padding:8px 14px;border:1px solid #ddd;border-radius:8px;background:#fafafa;cursor:pointer', '取消');
  var submit = el('button', 'padding:8px 14px;border:none;border-radius:8px;background:' + EMBER + ';color:#fff;font-weight:600;cursor:pointer', '提交');

  bodyWrap.appendChild(title); bodyWrap.appendChild(desc); bodyWrap.appendChild(msg);
  actions.appendChild(cancel); actions.appendChild(submit);
  card.appendChild(head); card.appendChild(bodyWrap); card.appendChild(actions);
  overlay.appendChild(card); document.body.appendChild(overlay);

  function open() { overlay.style.display = 'flex'; title.focus(); }
  function close() { overlay.style.display = 'none'; msg.textContent = ''; }
  btn.addEventListener('click', open);
  cancel.addEventListener('click', close);
  overlay.addEventListener('click', function (e) { if (e.target === overlay) close(); });

  submit.addEventListener('click', function () {
    if (!title.value.trim()) { msg.textContent = '请填写标题'; return; }
    submit.disabled = true; msg.textContent = '提交中…';
    var headers = { 'Content-Type': 'application/json' };
    if (token) headers['Authorization'] = 'Bearer ' + token;
    fetch(endpoint + '/webhook/issues', {
      method: 'POST',
      headers: headers,
      body: JSON.stringify({
        project_id: project,
        title: title.value.trim(),
        description: desc.value.trim(),
        category: 'Feature',
        severity: 'medium',
        source_ref: location.href
      })
    }).then(function (r) {
      submit.disabled = false;
      if (r.ok) { msg.textContent = '已提交，谢谢！'; title.value = ''; desc.value = ''; setTimeout(close, 900); }
      else { msg.textContent = '提交失败（' + r.status + '）'; }
    }).catch(function () { submit.disabled = false; msg.textContent = '网络错误'; });
  });
})();
