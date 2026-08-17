/**
 * 数据源的取数与缓存。
 *
 * 块自己不发请求——它们声明一个 binding，由这里统一取数并分发。
 * 于是同一个来源被十个块引用也只打一次后端，刷新也只需要一处。
 *
 * 缓存按 **来源 + 参数** 分键：GST 的 1 月期和 3 月期是两份独立的数据，
 * 切换页面参数时旧数据留在缓存里，切回去不用重取。
 */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { DATA_SOURCES, isDataSource } from "./ipc";
import { sourceKey } from "./types";

export type SourceState = {
  data: unknown;
  loading: boolean;
  error: string | null;
};

const EMPTY: SourceState = { data: undefined, loading: false, error: null };

type DataContextValue = {
  /** 读缓存。不触发请求。 */
  get: (source: string, params: Record<string, string>) => SourceState;
  /** 确保这份数据被取过一次。同一个键重复调用不会重复打后端。 */
  ensure: (source: string, params: Record<string, string>) => void;
  /** 强制重取某一份。 */
  refresh: (source: string, params: Record<string, string>) => void;
  /** 重取当前已经被请求过的所有数据。 */
  refreshAll: () => void;
};

const DataContext = createContext<DataContextValue>({
  get: () => EMPTY,
  ensure: () => {},
  refresh: () => {},
  refreshAll: () => {},
});

export function DataProvider({ children }: { children: ReactNode }) {
  const [states, setStates] = useState<Record<string, SourceState>>({});
  /** 已经请求过的键 → 参数。refreshAll 靠它知道该重取什么。 */
  const requested = useRef(new Map<string, { source: string; params: Record<string, string> }>());

  const fetchKey = useCallback((source: string, params: Record<string, string>) => {
    const key = sourceKey(source, params);
    requested.current.set(key, { source, params });

    if (!isDataSource(source)) {
      setStates((prev) => ({
        ...prev,
        [key]: { data: undefined, loading: false, error: `未知的数据源 ${source}` },
      }));
      return;
    }

    setStates((prev) => ({ ...prev, [key]: { data: prev[key]?.data, loading: true, error: null } }));
    DATA_SOURCES[source](params)
      .then((data) => setStates((prev) => ({ ...prev, [key]: { data, loading: false, error: null } })))
      .catch((e: unknown) =>
        setStates((prev) => ({
          ...prev,
          // 保留上一次的数据：一次失败不该让整屏数字消失。
          [key]: { data: prev[key]?.data, loading: false, error: String(e) },
        })),
      );
  }, []);

  const ensure = useCallback(
    (source: string, params: Record<string, string>) => {
      if (requested.current.has(sourceKey(source, params))) return;
      fetchKey(source, params);
    },
    [fetchKey],
  );

  const refreshAll = useCallback(() => {
    for (const { source, params } of [...requested.current.values()]) fetchKey(source, params);
  }, [fetchKey]);

  const value = useMemo<DataContextValue>(
    () => ({
      get: (source, params) => states[sourceKey(source, params)] ?? EMPTY,
      ensure,
      refresh: fetchKey,
      refreshAll,
    }),
    [states, ensure, fetchKey, refreshAll],
  );

  return <DataContext.Provider value={value}>{children}</DataContext.Provider>;
}

export function useData(): DataContextValue {
  return useContext(DataContext);
}

/**
 * 订阅一份数据：挂载时（以及键变化时）确保取过一次，然后返回它的状态。
 * `ensure` 幂等，所以同一份数据被多个块订阅也只打一次后端。
 */
export function useSource(source: string | null, params: Record<string, string>): SourceState {
  const data = useData();
  const key = source ? sourceKey(source, params) : "";

  useEffect(() => {
    if (source) data.ensure(source, params);
    // params 已经被压进 key，用它做依赖，避免每次渲染新建对象触发重取。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, key]);

  return source ? data.get(source, params) : EMPTY;
}
