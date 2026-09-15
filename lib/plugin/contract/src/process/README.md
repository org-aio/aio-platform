# Process 宿主协议

Configuration 由宿主写入私有只读挂载，包含当前租户的数据角色、派生密钥、入口票据和 Unix broker 地址。模型出站与跨插件调用由宿主执行并重新鉴权。ServiceRequest 的交互身份只在对应入站请求有效期内有效，后台调用使用独立服务身份。

`model_endpoint` 统一校验模型基址：允许 HTTPS，以及显式声明的 RFC1918 / IPv6 ULA 私网 IP 的 HTTP 地址；拒绝 HTTP 域名、公网、回环和链路本地地址。插件声明与宿主 `AIO_PROCESS_ENDPOINTS` 必须同时批准完整基址，校验通过不等于授予出站权限。
