import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import {
  THEME_SYSTEM,
  getPreferences,
  updatePreferences,
} from "@/api/preferences";
import {
  applyNativeTheme,
  readSystemAppearance,
  subscribeSystemAppearance,
} from "./host";
import { resolveAppearance } from "./theme";

const PreferencesContext = createContext(null);

/**
 * 偏好上下文：当前偏好、解析后的外观、切换主题的方法。
 *
 * @returns {{
 *   preferences: import("@/api/preferences").Preferences,
 *   appearance: import("@/api/preferences").Appearance,
 *   setTheme: (theme: import("@/api/preferences").ThemePreference) => Promise<void>,
 * }}
 */
export function usePreferences() {
  const value = useContext(PreferencesContext);
  if (!value) {
    throw new Error("usePreferences 必须在 PreferencesProvider 之内使用");
  }
  return value;
}

/**
 * 偏好上下文提供者：加载偏好、跟踪系统外观，并把主题偏好解析成应用外观。
 *
 * 位于 `ConfigProvider` 之上：antd 的明暗算法、侧边栏外观与原生控件都跟着它
 * 解析出来的外观走，故整窗一起变，而不是只有部分区域变。
 */
export default function PreferencesProvider({ children }) {
  const [preferences, setPreferences] = useState(null);
  const [systemAppearance, setSystemAppearance] = useState("light");

  // 加载偏好与系统外观。读取失败按缺省值运行：偏好是外观问题，不该把应用堵住。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [loaded, appearance] = await Promise.all([
        getPreferences().catch((err) => {
          console.warn("failed to load preferences", err);
          // 兜底只需主题：版本由后端写盘时强制，前端不必知道 schema 版本。
          return { theme: THEME_SYSTEM };
        }),
        readSystemAppearance(),
      ]);
      if (cancelled) return;
      setPreferences(loaded);
      setSystemAppearance(appearance);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const theme = preferences?.theme;
  const appearance = useMemo(
    () => resolveAppearance(theme ?? THEME_SYSTEM, systemAppearance),
    [theme, systemAppearance],
  );

  // 「跟随系统」时订阅系统外观变化；解析结果与当前值相同时不触发重渲染。
  useEffect(() => {
    if (theme !== THEME_SYSTEM) return undefined;
    return subscribeSystemAppearance((next) => {
      setSystemAppearance((current) => (current === next ? current : next));
    });
  }, [theme]);

  // 原生控件跟随：把解析后的外观写到根元素，滚动条、表单控件等随之变化。
  // 偏好尚未加载时不写——那会把「跟随系统」提前钉成亮色，反而闪一下。
  useEffect(() => {
    if (!preferences) return;
    document.documentElement.style.colorScheme = appearance;
  }, [preferences, appearance]);

  // 原生标题栏与控件跟随：选亮色 / 暗色时显式设置，选跟随系统时交还系统。
  //
  // 交还之后才读系统外观：窗口主题交还前由应用定住，读到的会是应用自己的
  // 选择而不是系统外观。读完再落地，前端与原生控件才对得上。
  useEffect(() => {
    if (!preferences) return undefined;
    let cancelled = false;
    (async () => {
      await applyNativeTheme(preferences.theme);
      if (cancelled || preferences.theme !== THEME_SYSTEM) return;
      const appearance = await readSystemAppearance();
      if (cancelled) return;
      setSystemAppearance((current) =>
        current === appearance ? current : appearance,
      );
    })();
    return () => {
      cancelled = true;
    };
  }, [preferences]);

  /**
   * 切换主题：即时生效并写盘，无保存按钮。
   *
   * 写盘失败时回滚到切换前的值并把原因抛给调用方——界面不展示一个没被保存的
   * 状态，提示由界面层给出（提示要落在 `ConfigProvider` 之内，故不在这里弹）。
   */
  const setTheme = useCallback(
    async (next) => {
      const previous = preferences;
      if (!previous || previous.theme === next) return;
      setPreferences({ ...previous, theme: next });
      try {
        await updatePreferences({ ...previous, theme: next });
      } catch (err) {
        setPreferences(previous);
        throw err;
      }
    },
    [preferences],
  );

  const value = useMemo(
    () => (preferences ? { preferences, appearance, setTheme } : null),
    [preferences, appearance, setTheme],
  );

  // 门控首帧：偏好加载完成前不渲染主界面，避免「先亮后暗」闪一下。
  if (!value) return null;

  return (
    <PreferencesContext.Provider value={value}>
      {children}
    </PreferencesContext.Provider>
  );
}
