# ZXTouch 设备身份接口待补充

用户已确认：设备 GUID 与虚拟 App 的 IDFV 是不同层级的身份。

- `/api/device/info` 应补充完整真实设备 GUID 返回字段，不由 RV 根据 IP 或本地 UUID 生成。
- 设备表使用本地 `id` 作主键，GUID 作唯一识别字段。接口补充前 GUID 留空；不展示 GUID。
- 现有 `identifierForVendor` 不可写入设备 GUID。
- IDFV 表示虚拟 App 在设备上的身份。同一设备、同一 Bundle 的不同虚拟 App 可以具有不同 IDFV。
- 虚拟 App 主键直接使用真实 IDFV，不另外生成本地主键；IDFV 统一规范格式后去重。不以 `(device, bundle)` 限制业务对象数量。
- 未读取到 IDFV 的备份不能自动生成虚拟 App 卡片。同 IDFV 重复导入关联到已有对象，不新增重复对象。

此文记录手机端待补充事项，不表示当前接口已经提供真实 GUID。
