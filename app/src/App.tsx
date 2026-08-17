import { useEffect, useState } from "react";

// 副作用 import：每个块在自己文件末尾 registerBlock，这一行完成全部注册。
// 必须在渲染器之前执行。
import "./blocks";

import { ChatPanel } from "./agent/ChatPanel";
import { useChat } from "./agent/store";
import { DataProvider } from "./core/data";
import { PageParamsBar } from "./core/PageParamsBar";
import { PageRenderer } from "./core/Renderer";
import { useEditor } from "./core/store";
import { Inspector } from "./editor/Inspector";
import { PageBar } from "./editor/PageBar";
import { Palette } from "./editor/Palette";
import { Toolbar } from "./editor/Toolbar";
import { SettingsDialog } from "./settings/SettingsDialog";

export default function App() {
  const status = useEditor((s) => s.status);
  const error = useEditor((s) => s.error);
  const source = useEditor((s) => s.source);
  const mode = useEditor((s) => s.mode);
  const breakpoint = useEditor((s) => s.breakpoint);
  const savedVersion = useEditor((s) => s.savedVersion);
  const pageId = useEditor((s) => s.pageId);
  const init = useEditor((s) => s.init);
  const initChat = useChat((s) => s.init);

  // 对话是主入口，默认打开。
  const [chatOpen, setChatOpen] = useState(true);

  useEffect(() => {
    void init();
    // 与面板开关无关：事件要一直收着，否则关掉面板就丢掉正在流的回复。
    void initChat();
  }, [init, initChat]);

  if (status === "loading") {
    return <div className="boot">启动中…</div>;
  }

  return (
    <DataProvider>
      <div className={`app mode-${mode}`}>
        <Toolbar chatOpen={chatOpen} onToggleChat={() => setChatOpen((v) => !v)} />
        <PageBar />

        <main className="workspace">
          <div className={`canvas breakpoint-${breakpoint}`}>
            <PageParamsBar pageId={pageId} />
            <PageRenderer />
          </div>

          {/*
            侧栏一次只有一个主人。对话是改界面的主入口；手动编辑面板留着，
            但退到 mode === "edit" 后面——它现在的主要价值是 agent 那条路径的
            底层机制（同一份布局文档、同一个撤销栈），不是日常操作方式。
          */}
          {mode === "edit" ? (
            <div className="sidebar">
              <Palette />
              <Inspector />
            </div>
          ) : chatOpen ? (
            <div className="sidebar">
              <ChatPanel />
            </div>
          ) : null}
        </main>

        <footer className="statusbar">
          <span className="status-source">{source}</span>
          {savedVersion !== null ? <span className="status-saved">布局 v{savedVersion} 已保存</span> : null}
          {error ? <span className="status-error">{error}</span> : null}
        </footer>

        <SettingsDialog />
      </div>
    </DataProvider>
  );
}
