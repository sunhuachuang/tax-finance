/**
 * 设置状态。
 *
 * 这里**不保存 key 原文**——`get_settings` 返回的本来就只有「有没有」和掩码提示。
 * 输入框里的字符串只在你点保存的那一刻存在，发出去之后就清掉。
 */
import { create } from "zustand";

import * as ipc from "../core/ipc";
import { useChat } from "../agent/store";

export type SettingsView = {
  has_api_key: boolean;
  api_key_hint: string | null;
  api_key_from_env: boolean;
  finance_host: string | null;
  host_from_env: boolean;
  llm_base_url: string | null;
  llm_model: string | null;
};

type SettingsStore = {
  open: boolean;
  view: SettingsView | null;
  saving: boolean;
  error: string | null;

  show: () => Promise<void>;
  hide: () => void;
  saveApiKey: (key: string) => Promise<void>;
  saveHost: (host: string) => Promise<void>;
  saveLlmEndpoint: (baseUrl: string, model: string) => Promise<void>;
};

export const useSettings = create<SettingsStore>((set) => ({
  open: false,
  view: null,
  saving: false,
  error: null,

  async show() {
    set({ open: true, error: null });
    try {
      set({ view: (await ipc.getSettings()) as SettingsView });
    } catch (e) {
      set({ error: `读取设置失败：${String(e)}` });
    }
  },

  hide: () => set({ open: false, error: null }),

  async saveApiKey(key) {
    set({ saving: true, error: null });
    try {
      const view = (await ipc.setApiKey(key)) as SettingsView;
      set({ view, saving: false });
      // 助手是即时生效的（Rust 侧按新 key 重建了会话），
      // 让聊天面板重新问一次状态，横幅就消失了。
      await useChat.getState().init();
    } catch (e) {
      set({ saving: false, error: `保存失败：${String(e)}` });
    }
  },

  async saveLlmEndpoint(baseUrl, model) {
    set({ saving: true, error: null });
    try {
      const view = (await ipc.setLlmEndpoint(baseUrl, model)) as SettingsView;
      set({ view, saving: false });
      // 换供应商后助手是按新配置重建的，让面板重新问一次状态。
      await useChat.getState().init();
    } catch (e) {
      set({ saving: false, error: `保存失败：${String(e)}` });
    }
  },

  async saveHost(host) {
    set({ saving: true, error: null });
    try {
      set({ view: (await ipc.setFinanceHost(host)) as SettingsView, saving: false });
    } catch (e) {
      set({ saving: false, error: `保存失败：${String(e)}` });
    }
  },
}));
