import { Button, Flex, message, Tag, Tooltip, Typography } from "antd";
import { FileTextOutlined } from "@ant-design/icons";
import { openPath } from "@tauri-apps/plugin-opener";
import { applyStatusPresentation } from "./applyStatus";

/**
 * 逐插件投影结果（见 issue #34 用户故事 2/3/4）。
 * @param {Object} param0
 * @param {import("@/api/plugins").PluginApplyReport[]} param0.reports
 */
export default function ApplyResultList({ reports }) {
  return (
    <Flex vertical gap={12}>
      {reports.map((report) => {
        const presentation = applyStatusPresentation(report.status);
        return (
          <div key={report.id}>
            <Flex align="center" gap={8}>
              <Typography.Text strong>{report.id}</Typography.Text>
              {presentation && (
                <Tag color={presentation.color}>{presentation.label}</Tag>
              )}
            </Flex>
            {report.files.length > 0 && (
              <Flex align="center" gap={8}>
                <Typography.Paragraph type="secondary" style={{ margin: 0 }}>
                  写入文件：{report.files.join("、")}
                </Typography.Paragraph>
                <Tooltip title="打开文件">
                  <Button
                    size="small"
                    shape="circle"
                    aria-label="打开文件"
                    icon={<FileTextOutlined />}
                    onClick={async () => {
                      try {
                        await openPath(report.files[0]);
                      } catch (err) {
                        // opener 是插件命令，失败不经 Rust 命令层，前端自行落盘（ADR 0008）。
                        console.error(String(err));
                        message.error(String(err));
                      }
                    }}
                  />
                </Tooltip>
              </Flex>
            )}
            {report.skipped.map((skip) => (
              <Typography.Paragraph
                type="secondary"
                style={{ margin: 0 }}
                key={skip.slug}
              >
                跳过 Provider「{skip.slug}」：{skip.reason}
              </Typography.Paragraph>
            ))}
            {report.reason && (
              <Typography.Paragraph type="danger" style={{ margin: 0 }}>
                {report.reason}
              </Typography.Paragraph>
            )}
          </div>
        );
      })}
    </Flex>
  );
}
