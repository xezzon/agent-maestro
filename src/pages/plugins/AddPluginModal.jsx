import { useState } from "react";
import {
  Button,
  Form,
  Input,
  message,
  Modal,
  Radio,
} from "antd";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

const URL_RULES = [
  { required: true, message: "请输入 manifest.json 的 https 地址" },
  {
    validator: (_, value) => {
      if (!value) return Promise.resolve();
      let url;
      try {
        url = new URL(value);
      } catch {
        return Promise.reject(new Error("不是合法的 URL"));
      }
      if (url.protocol !== "https:") {
        return Promise.reject(new Error("插件来源仅支持 https 地址"));
      }
      return Promise.resolve();
    },
  },
];

const PATH_RULES = [{ required: true, message: "请选择插件的 manifest.json" }];

/** 来源种类：文案与来源模型的命名一致（不出现「Git」）。 */
const SOURCE_OPTIONS = [
  { label: "https 地址", value: "https" },
  { label: "本地文件", value: "file" },
];

/**
 * 添加插件对话框：radio 切换来源种类——「https 地址」为默认（联动 URL 输入框），
 * 「本地文件」联动文件选择器选中本机 manifest.json（插件作者的本地调试回路）。
 *
 * @param {Object} param0
 * @param {boolean} param0.open
 * @param {() => void} param0.onClose
 * @param {(source: string) => Promise<void>} param0.onSubmit
 */
export default function AddPluginModal({ open, onClose, onSubmit }) {
  const [form] = Form.useForm();
  const [kind, setKind] = useState("https");
  const [picking, setPicking] = useState(false);
  const [saving, setSaving] = useState(false);

  /** 关闭即复位：每次打开都回到默认来源与空输入。 */
  function close() {
    form.resetFields();
    setKind("https");
    onClose();
  }

  async function pickManifest() {
    setPicking(true);
    try {
      const selected = await openDialog({
        title: "选择插件的 manifest.json",
        multiple: false,
        directory: false,
        filters: [{ name: "manifest.json", extensions: ["json"] }],
      });
      if (typeof selected === "string") {
        form.setFieldValue("path", selected);
      }
    } catch (err) {
      message.error(String(err));
    } finally {
      setPicking(false);
    }
  }

  async function handleSubmit() {
    let values;
    try {
      values = await form.validateFields();
    } catch {
      // 校验失败已内联展示。
      return;
    }
    const source = kind === "https" ? values.url : values.path;
    setSaving(true);
    try {
      await onSubmit(source);
      close();
    } catch (err) {
      message.error(String(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal
      title="添加插件"
      open={open}
      okText="添加"
      cancelText="取消"
      confirmLoading={saving}
      destroyOnHidden
      onOk={handleSubmit}
      onCancel={close}
    >
      <Form form={form} layout="vertical" onFinish={handleSubmit}>
        <Form.Item label="来源">
          <Radio.Group
            options={SOURCE_OPTIONS}
            value={kind}
            disabled={saving}
            onChange={(e) => setKind(e.target.value)}
          />
        </Form.Item>
        {kind === "https" ? (
          <Form.Item
            name="url"
            label="https 地址"
            extra="指向插件 manifest.json 的 https 地址，例如 GitHub Release 上的 manifest.json"
            rules={URL_RULES}
          >
            <Input placeholder="https://github.com/owner/repo/releases/download/v1/manifest.json" />
          </Form.Item>
        ) : (
          <>
            <Form.Item
              name="path"
              label="本地 manifest.json"
              extra="插件项目根目录下的 manifest.json；entry 为相对路径时读该目录内的产物，为 https 地址时联网获取"
              rules={PATH_RULES}
            >
              <Input readOnly placeholder="选择本机插件项目的 manifest.json" />
            </Form.Item>
            <Form.Item style={{ marginBottom: 0 }}>
              <Button loading={picking} disabled={saving} onClick={pickManifest}>
                选择文件
              </Button>
            </Form.Item>
          </>
        )}
      </Form>
    </Modal>
  );
}
