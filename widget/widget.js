/**
 * AutoForge Feedback Widget
 * Embeds a feedback button in any web page with one script tag.
 *
 * Usage:
 *   <script src="https://your-autoforge.io/widget/widget.js"
 *           data-project-id="your-project-uuid"
 *           data-api-key="af_xxxx"
 *           data-api-url="https://your-autoforge.io">
 *   </script>
 */
(function () {
  'use strict';

  const script = document.currentScript;
  const PROJECT_ID = script.getAttribute('data-project-id');
  const API_KEY = script.getAttribute('data-api-key');
  const API_URL = (script.getAttribute('data-api-url') || '').replace(/\/$/, '');

  if (!PROJECT_ID || !API_URL) {
    console.warn('[AutoForge Widget] Missing data-project-id or data-api-url');
    return;
  }

  // Inject styles
  const style = document.createElement('style');
  style.textContent = `
    #af-widget-btn {
      position: fixed; bottom: 24px; right: 24px; z-index: 99999;
      width: 48px; height: 48px; border-radius: 50%;
      background: #6c8ef5; color: #fff; border: none; cursor: pointer;
      font-size: 22px; box-shadow: 0 4px 14px rgba(108,142,245,0.4);
      transition: transform 0.15s, box-shadow 0.15s;
    }
    #af-widget-btn:hover { transform: scale(1.1); box-shadow: 0 6px 20px rgba(108,142,245,0.5); }
    #af-overlay {
      display: none; position: fixed; inset: 0; background: rgba(0,0,0,0.5);
      z-index: 99998; align-items: center; justify-content: center;
    }
    #af-overlay.open { display: flex; }
    #af-modal {
      background: #1a1d27; border: 1px solid #2a2d3e; border-radius: 12px;
      width: 400px; max-width: 95vw; padding: 24px; color: #e8eaf0;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    #af-modal h2 { font-size: 16px; font-weight: 700; margin: 0 0 16px; }
    #af-modal label { display: block; font-size: 12px; color: #8b8fa8; margin-bottom: 4px; margin-top: 12px; }
    #af-modal input, #af-modal textarea, #af-modal select {
      width: 100%; background: #0f1117; border: 1px solid #2a2d3e; border-radius: 6px;
      padding: 8px 10px; color: #e8eaf0; font-size: 13px; box-sizing: border-box;
      resize: vertical;
    }
    #af-modal .af-row { display: flex; gap: 12px; }
    #af-modal .af-row > * { flex: 1; }
    #af-modal .af-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 20px; }
    #af-modal button { padding: 8px 16px; border-radius: 6px; font-size: 13px; cursor: pointer; border: none; }
    #af-cancel-btn { background: transparent; border: 1px solid #2a2d3e; color: #e8eaf0; }
    #af-submit-btn { background: #6c8ef5; color: #fff; }
    #af-submit-btn:disabled { opacity: 0.5; cursor: not-allowed; }
    #af-success { text-align: center; padding: 20px 0; }
    #af-success .af-check { font-size: 40px; }
    #af-success p { margin-top: 12px; color: #8b8fa8; font-size: 14px; }
  `;
  document.head.appendChild(style);

  // Widget button
  const btn = document.createElement('button');
  btn.id = 'af-widget-btn';
  btn.title = '提交反馈';
  btn.textContent = '💬';
  document.body.appendChild(btn);

  // Modal overlay
  const overlay = document.createElement('div');
  overlay.id = 'af-overlay';
  overlay.innerHTML = `
    <div id="af-modal">
      <h2>提交反馈</h2>
      <div id="af-form-body">
        <label>问题描述 *</label>
        <input id="af-title" type="text" placeholder="用一句话描述问题或建议..." maxlength="200">
        <label>详细说明（可选）</label>
        <textarea id="af-desc" rows="3" placeholder="复现步骤、期望效果等..."></textarea>
        <div class="af-row">
          <div>
            <label>类型</label>
            <select id="af-category">
              <option value="Bug">Bug</option>
              <option value="Feature">功能建议</option>
              <option value="Improvement">体验改进</option>
            </select>
          </div>
          <div>
            <label>严重程度</label>
            <select id="af-severity">
              <option value="low">低</option>
              <option value="medium" selected>中</option>
              <option value="high">高</option>
            </select>
          </div>
        </div>
        <div class="af-actions">
          <button id="af-cancel-btn">取消</button>
          <button id="af-submit-btn">提交</button>
        </div>
      </div>
      <div id="af-success" style="display:none">
        <div class="af-check">✓</div>
        <p>感谢您的反馈！<br>我们已收到并将尽快处理。</p>
      </div>
    </div>
  `;
  document.body.appendChild(overlay);

  const modal = document.getElementById('af-modal');
  const cancelBtn = document.getElementById('af-cancel-btn');
  const submitBtn = document.getElementById('af-submit-btn');
  const successDiv = document.getElementById('af-success');
  const formBody = document.getElementById('af-form-body');

  function open() { overlay.classList.add('open'); }
  function close() {
    overlay.classList.remove('open');
    // Reset form
    document.getElementById('af-title').value = '';
    document.getElementById('af-desc').value = '';
    formBody.style.display = '';
    successDiv.style.display = 'none';
  }

  btn.addEventListener('click', open);
  cancelBtn.addEventListener('click', close);
  overlay.addEventListener('click', function (e) { if (e.target === overlay) close(); });

  submitBtn.addEventListener('click', async function () {
    const title = document.getElementById('af-title').value.trim();
    if (!title) { document.getElementById('af-title').focus(); return; }

    submitBtn.disabled = true;
    submitBtn.textContent = '提交中...';

    try {
      const resp = await fetch(API_URL + '/api/v1/issues', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-Api-Key': API_KEY || '',
          'X-Source-URL': location.href,
        },
        body: JSON.stringify({
          project_id: PROJECT_ID,
          source_type: 'widget',
          title: title,
          description: document.getElementById('af-desc').value.trim() || null,
          category: document.getElementById('af-category').value,
          severity: document.getElementById('af-severity').value,
          source_url: location.href,
        }),
      });

      if (resp.ok || resp.status === 409) {
        formBody.style.display = 'none';
        successDiv.style.display = '';
        setTimeout(close, 3000);
      } else {
        alert('提交失败，请稍后重试');
      }
    } catch (err) {
      alert('网络错误，请检查连接后重试');
    } finally {
      submitBtn.disabled = false;
      submitBtn.textContent = '提交';
    }
  });
})();
