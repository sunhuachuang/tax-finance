/**
 * 设置。API key 和远程 host 在这里填，不再需要环境变量。
 *
 * key 只在你输入的那一刻穿过页面一次，保存后立刻从输入框清掉；
 * 读设置时拿回来的是掩码，不是原文。
 */
import { useEffect, useState } from "react";

import { useSettings } from "./store";

/** 已知提供 Anthropic 兼容端点的服务。点一下填好地址和模型名，省得去翻文档。 */
const PRESETS = [
  { label: "Anthropic", baseUrl: "", model: "" },
  { label: "Haiku（便宜 5 倍）", baseUrl: "", model: "claude-haiku-4-5" },
  { label: "DeepSeek", baseUrl: "https://api.deepseek.com/anthropic", model: "deepseek-v4-flash" },
  { label: "Kimi", baseUrl: "https://api.moonshot.ai/anthropic", model: "kimi-k2-turbo-preview" },
  { label: "GLM", baseUrl: "https://api.z.ai/api/anthropic", model: "glm-4.6" },
  { label: "MiniMax", baseUrl: "https://api.minimax.io/anthropic", model: "MiniMax-M2" },
];

export function SettingsDialog() {
  const open = useSettings((s) => s.open);
  const view = useSettings((s) => s.view);
  const saving = useSettings((s) => s.saving);
  const error = useSettings((s) => s.error);
  const hide = useSettings((s) => s.hide);
  const saveApiKey = useSettings((s) => s.saveApiKey);
  const saveHost = useSettings((s) => s.saveHost);
  const saveLlmEndpoint = useSettings((s) => s.saveLlmEndpoint);

  const [key, setKey] = useState("");
  const [host, setHost] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");

  useEffect(() => {
    if (open) setKey("");
  }, [open]);

  useEffect(() => {
    setHost(view?.finance_host ?? "");
  }, [view?.finance_host]);

  useEffect(() => {
    setBaseUrl(view?.llm_base_url ?? "");
    setModel(view?.llm_model ?? "");
  }, [view?.llm_base_url, view?.llm_model]);

  useEffect(() => {
    if (!open) return;
    const onEsc = (e: KeyboardEvent) => e.key === "Escape" && hide();
    window.addEventListener("keydown", onEsc);
    return () => window.removeEventListener("keydown", onEsc);
  }, [open, hide]);

  if (!open) return null;

  async function submitKey() {
    await saveApiKey(key);
    // 存完就从页面上抹掉，不留在 React state 里。
    setKey("");
  }

  return (
    <div className="modal-backdrop" onClick={hide}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">
          设置
          <button type="button" className="modal-close" onClick={hide} aria-label="关闭">
            ×
          </button>
        </div>

        {error ? <div className="block-error">{error}</div> : null}

        <section className="modal-section">
          <div className="panel-label">模型服务</div>

          <p className="settings-current">
            当前：<code>{view?.llm_base_url ?? "Anthropic 官方"}</code>
            <code>{view?.llm_model ?? "claude-opus-5"}</code>
          </p>
          <p className="panel-hint">
            留空 = Anthropic 官方 + <code>claude-opus-5</code>。
            填第三方的 <strong>Anthropic 兼容端点</strong>就能换供应商——协议完全一样，
            便宜很多，适合测试。
          </p>

          <div className="preset-row">
            {PRESETS.map((preset) => (
              <button
                key={preset.label}
                type="button"
                className="btn subtle preset"
                onClick={() => {
                  setBaseUrl(preset.baseUrl);
                  setModel(preset.model);
                }}
              >
                {preset.label}
              </button>
            ))}
          </div>

          <label className="field">
            <span className="field-label">服务地址</span>
            <input
              type="text"
              value={baseUrl}
              placeholder="留空 = Anthropic 官方"
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => setBaseUrl(e.target.value)}
            />
          </label>

          <label className="field">
            <span className="field-label">模型名</span>
            <input
              type="text"
              value={model}
              placeholder="留空 = claude-opus-5"
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => setModel(e.target.value)}
            />
          </label>

          <div className="modal-actions">
            <button
              type="button"
              className="btn"
              disabled={
                saving ||
                // 填了地址却没填模型名是配置不全：第三方不认识 Anthropic 的模型名。
                (baseUrl.trim() !== "" && model.trim() === "") ||
                (baseUrl === (view?.llm_base_url ?? "") && model === (view?.llm_model ?? ""))
              }
              onClick={() => void saveLlmEndpoint(baseUrl, model)}
            >
              保存
            </button>
          </div>

          {baseUrl.trim() !== "" && model.trim() === "" ? (
            <p className="panel-hint settings-warn">
              填了服务地址就必须填模型名——第三方服务不认识 Anthropic 的模型名。
            </p>
          ) : null}

          <p className="panel-hint">
            换服务后 <strong>API key 也要换成对应服务的</strong>（在下面那一栏填）。
            第三方端点不支持 Anthropic 专有的 thinking 和 effort，助手会自动不发这两个字段。
          </p>
        </section>

        <section className="modal-section">
          <div className="panel-label">Claude API Key</div>

          {view?.has_api_key ? (
            <p className="settings-current">
              已设置 <code>{view.api_key_hint}</code>
              {view.api_key_from_env ? (
                <span className="settings-badge">来自环境变量</span>
              ) : (
                <span className="settings-badge saved">已存盘</span>
              )}
            </p>
          ) : (
            <p className="panel-hint">还没有设置，AI 助手不可用。</p>
          )}

          {/* 环境变量优先。存了却不生效是最容易困惑的情况，说清楚。 */}
          {view?.api_key_from_env ? (
            <p className="panel-hint">
              环境变量 <code>ANTHROPIC_API_KEY</code> 优先于这里存的值。
              要用存盘的 key，先取消那个环境变量再重启。
            </p>
          ) : null}

          <label className="field">
            <span className="field-label">新的 key</span>
            <input
              type="password"
              value={key}
              placeholder="sk-ant-..."
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => setKey(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void submitKey();
              }}
            />
          </label>

          <div className="modal-actions">
            <button
              type="button"
              className="btn active"
              disabled={saving || !key.trim()}
              onClick={() => void submitKey()}
            >
              {saving ? "保存中…" : "保存"}
            </button>
            {view?.has_api_key && !view.api_key_from_env ? (
              <button
                type="button"
                className="btn danger"
                disabled={saving}
                onClick={() => void saveApiKey("")}
              >
                清除
              </button>
            ) : null}
          </div>

          <p className="panel-hint">
            到 <code>console.anthropic.com</code> 的 API Keys 页面创建，
            形如 <code>sk-ant-api03-…</code>。
          </p>
          <p className="panel-hint">
            <strong>不要用 claude.ai 的登录凭证</strong>：网页会话 key
            （<code>sk-ant-sid…</code>）不能用于 API；OAuth token
            （<code>sk-ant-oat…</code>）能用但会过期，只适合临时试。
          </p>
          <p className="panel-hint">
            存在 <code>settings.json</code>（应用数据目录，权限 0600），
            和账本同一个信任级别。保存后立刻生效，不用重启。
          </p>
        </section>

        <section className="modal-section">
          <div className="panel-label">远程 host</div>
          <p className="panel-hint">
            填了就切到远程模式：账本在那台机器上，本机不存副本。
            移动端必须填。留空 = 用本机账本。
          </p>

          <label className="field">
            <span className="field-label">地址</span>
            <input
              type="text"
              value={host}
              placeholder="http://mac-mini:5710"
              autoComplete="off"
              spellCheck={false}
              disabled={view?.host_from_env}
              onChange={(e) => setHost(e.target.value)}
            />
          </label>

          <div className="modal-actions">
            <button
              type="button"
              className="btn"
              disabled={saving || view?.host_from_env || host === (view?.finance_host ?? "")}
              onClick={() => void saveHost(host)}
            >
              保存
            </button>
          </div>

          {view?.host_from_env ? (
            <p className="panel-hint">
              当前由环境变量 <code>FINANCE_HOST</code> 指定，这里改不动。
            </p>
          ) : (
            <p className="panel-hint">
              <strong>改完要重启应用才生效</strong>——数据来源在启动时就定下来了。
            </p>
          )}

          <p className="panel-hint">
            这条链路必须跑在 Tailscale / WireGuard 隧道里，不要暴露到公网。
          </p>
        </section>
      </div>
    </div>
  );
}
