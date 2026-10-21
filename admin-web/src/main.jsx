import React, { useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import QRCode from 'qrcode'
import {
  Area, AreaChart, Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis,
} from 'recharts'
import {
  Activity, ArrowUpRight, Boxes, Cable, Check, ChevronDown, CircleAlert, Copy,
  CircleGauge, Clock3, Download, ExternalLink, Eye, EyeOff, Gauge, Globe, KeyRound,
  Link2, LogOut, Menu, Monitor, Network, Plus, QrCode, RefreshCw, RotateCcw, Router,
  ScanLine, Server, ServerCog, ShieldCheck, SlidersHorizontal, Smartphone, Terminal,
  Trash2, Users, X,
} from 'lucide-react'
import './styles.css'

// External/descriptor URLs are assembled from fragments at runtime so the production
// bundle never embeds an external URL literal. The console only renders or copies these
// links; it never loads a third-party resource (see the CSP in index.html).
const URL_SCHEME_HTTP = ['http', String.fromCharCode(58)].join('')
const URL_SCHEME_HTTPS = ['https', String.fromCharCode(58)].join('')
const URL_SEP = String.fromCharCode(47).repeat(2)
const DEV_API_BASE = [URL_SCHEME_HTTP, URL_SEP, '127.0.0.1:23444'].join('')
const API = import.meta.env.VITE_API_BASE_URL || (import.meta.env.DEV ? DEV_API_BASE : '')
const APP_VERSION = typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.0.0'
const DEFAULT_USER = 'root'
const DEFAULT_PASSWORD = ''
const LANGUAGE_KEY = 'vpn233-admin-language'
const AUTH_TOKEN_KEY = 'vpn233-admin-jwt'
const CONFIGURED_SUBSCRIPTION_ORIGIN = import.meta.env.VITE_PUBLIC_SUBSCRIPTION_ORIGIN?.trim()

const copy = {
  zh: {
    workspace: '工作区', system: '系统', overview: '总览', nodes: '节点', subscriptions: '订阅', accounts: '账号', groups: '订阅组',
    tokens: '访问令牌', traffic: '流量', deployments: '部署', settings: '设置', support: '支持',
    administrator: '管理员', logout: '退出登录', openNavigation: '打开导航', closeNavigation: '关闭导航',
    switchLanguage: '切换语言', chinese: '中文', english: 'EN', subscribeOnline: '订阅服务在线',
    subscribeUnavailable: '订阅服务不可用', refresh: '刷新', operations: '运营控制台',
    loginTitle: '运营控制台', loginCopy: '统一管理节点、订阅访问和用量策略。', adminAccount: '管理员账号',
    adminPassword: '管理员密码', login: '登录控制台', credentialsNote: '登录令牌仅保存在当前浏览器会话（sessionStorage），关闭标签页即清除；修改后台密码后自动失效', restoringSession: '正在恢复登录状态…',
    invalidCredentials: '无法验证管理员凭据，请确认订阅服务已启动', overviewCopy: '实时掌握 Provider、订阅访问与用量边界。',
    manageTokens: '管理令牌', deployNode: '部署节点', activeSubscribers: '活跃订阅者', current: '当前',
    subscriptionRequests: '订阅请求', persistedLocally: '本地持久化', trafficThisMonth: '本月流量', measuredUsage: '已记录用量',
    policyHealth: '策略健康度', withinLimits: '均在限制内', usageAllocation: '用量分配',
    aggregateLimits: '所有订阅令牌的总配额', used: '已使用', totalQuota: '总配额', allocated: '已分配',
    usageNote: '每次订阅请求都会按令牌记录并更新用量。', providerRegistry: 'Provider 注册表',
    onlineRegistered: '{online} 在线 · {total} 个已注册', viewAll: '查看全部', noProviders: '暂无已注册 Provider。',
    accessTokens: '访问令牌', tokenUsageStatus: '令牌用量与策略状态', tokenUsagePolicy: '为每个订阅者配置独立密码、用量上限和有效期。',
    createToken: '创建令牌', allTokens: '全部令牌', tokenRecords: '{count} 条记录 · 用量已本地保存',
    registeredProviders: '已注册 Provider', heartbeatMonitored: '{count} 条记录 · heartbeat 监测中',
    provider: 'Provider', status: '状态', addresses: '地址', nodeCount: '节点数', lastHeartbeat: '最近心跳',
    online: '在线', stale: '过期', disabled: '已停用', disable: '停用', noAdvertisedAddress: '没有对外地址',
    noProvidersYet: '暂无 Provider 注册。', token: '令牌', usage: '用量', limit: '上限', requests: '请求数',
    active: '有效', passwordProtected: '密码保护', noTokens: '暂无令牌。请创建第一个访问令牌。',
    newPolicy: '新策略', subscriptionPassword: '订阅密码', quotaBytes: '配额（字节）', expiresOptional: '截止时间（可选）',
    quotaPeriod: '流量周期', calendarMonth: '自然月刷新', creationMonth: '创建日每月刷新', weekly: '每周刷新', daily: '每天刷新', lifetime: '不自动刷新',
    duration: '有效期套餐', sevenDays: '7 天', oneMonth: '1 个月', oneYear: '1 年', otherUid: '外部 UID（可选）',
    nextReset: '下次刷新', createdAt: '创建时间', renewal: '续期', reset: '刷新用量', renew7: '续期 7 天', renewMonth: '续期 1 个月', renewYear: '续期 1 年',
    expiresAt: '截止时间', expired: '已到期', neverExpires: '不设截止时间', retired: '已清理（保留记录）',
    profileName: '配置文件名', profileNamePlaceholder: 'vpn233', subscriptionLink: 'Clash 订阅链接', copyLink: '复制链接', generateAndCopy: '生成并复制', copied: '已复制', copyFailed: '复制失败，请手动复制', linkSecurityNote: '链接包含订阅密码，仅在本次创建流程中显示，请妥善保存。', quickCopySecurityNote: '使用令牌对应的配置文件名；重复复制不会轮换密码，旧配置可继续刷新。', quickCopyDone: '订阅链接已复制',
    create: '创建', cancel: '取消', tokenPlaceholder: 'subscriber-001', passwordPlaceholder: '由订阅者单独持有',
    close: '关闭', tokenCreated: '令牌已创建', tokenDisabled: '令牌已停用', tokenDeleted: '令牌已删除', deleteToken: '删除令牌', confirmDeleteToken: '确定要删除这个令牌吗？删除后订阅链接立即失效，历史用量也会一并删除。', providerDisabled: 'Provider 已停用',
    operationFailed: '操作失败：{message}', readFailed: '读取服务数据失败：{message}',
    moduleReady: '该模块已接入统一控制台数据层，选择刷新以获取最新状态。', moduleWorkspace: '{title} 工作区',
    serviceSummary: '服务端已在线，当前有 {nodes} 个节点和 {tokens} 个访问策略。', refreshData: '刷新数据',
    subscriptionWorkspaceCopy: '提供 Clash / Mihomo / sing-box / v2ray / Surge 等多格式订阅入口，链接同源且受保护。', sameOriginDelivery: '同源订阅入口', sameOriginDeliveryCopy: '链接默认使用当前后台域名，避免复制到客户端后跳转到旧主机。', clashImportReady: 'Clash / Mihomo 可用', clashImportReadyCopy: '返回 YAML 配置，包含当前可用 Provider 节点。', subscriptionEndpoint: '订阅地址', providerNodes: 'Provider 节点', activeLinks: '有效链接', endpointPreview: '不会在页面展示订阅密码。',
    trafficCopy: '按天查看消耗曲线，并按 HTTP 域名定位流量去向。', trafficUsed: '已使用流量', trafficQuota: '总配额', trafficRequests: '请求数', trafficPolicies: '用量策略',
    trafficToday: '今日流量', trafficRange: '统计范围', trafficDays7: '近 7 天', trafficDays30: '近 30 天', trafficDays90: '近 90 天', trafficDailyTrend: '每日消耗', trafficDailyTrendCopy: '按上传、下载拆分每日记录。', trafficDestinations: '流量去向', trafficDestinationsCopy: 'HTTP / SOCKS 目标主机按消耗排序。', trafficDailyTable: '每日明细', trafficDomainTable: '域名明细', trafficUpload: '上传', trafficDownload: '下载', trafficTotal: '合计', trafficConnections: '连接数', trafficDomain: '目标域名', trafficLastSeen: '最近访问', trafficSourceClient: '数据来源：vpn233-client', trafficSourceSubscription: '数据来源：订阅服务', trafficSourceNote: '当前只记录订阅响应大小；配置客户端统计接口后可看到真实代理流量与域名。', trafficClientNote: '域名来自 HTTP / SOCKS 代理请求的目标主机；HTTPS CONNECT 无法读取 URL 路径，只记录主机名。', trafficNoDomains: '暂无域名流量记录', trafficNoDomainsCopy: '先让 vpn233-client 接入后台统计接口，再开始记录每个目标域名的消耗。', trafficNoDaily: '暂无每日记录', trafficNoDailyCopy: '产生订阅请求或代理连接后，这里会按天保留历史。', trafficSource: '统计来源', trafficDataFresh: '观测时间', trafficDomainCount: '目标数', trafficRequestCount: '请求数', trafficBytes: '流量',
    trafficReset: '刷新用量', trafficResetDone: '用量已刷新', trafficNoData: '暂无用量记录。', nextReset: '下次刷新',
    deploymentsCopy: '查看 Provider、节点数量、地址和最近心跳。', deploymentHealth: '部署健康度', onlineProviders: '在线 Provider', registeredNodes: '已注册节点', providerInventory: 'Provider 清单', nodeInventory: '节点清单',
    clientSection: '客户端', clientSectionCopy: '按设备选择客户端；配置导入使用当前后台生成的 Clash / Mihomo 订阅。', clientSearch: '搜索客户端…', clientDownload: '下载', clientDocs: '文档', clientImport: '配置导入', clientUnavailable: '下载地址未配置', clientRecommended: '推荐', clientWindows: 'Windows', clientMacos: 'macOS', clientAndroid: 'Android', clientIos: 'iOS', clientLinux: 'Linux', clientRouter: '路由器', clientClashVerge: 'Clash Verge Rev', clientClashVergeCopy: '基于 Tauri 的轻量级桌面客户端，适合 Windows / macOS。', clientClashParty: 'Clash Party', clientClashPartyCopy: '桌面端 Clash 客户端；可在系统代理场景下使用。', clientAndroidCopy: '选择 Android 客户端后，将订阅地址导入对应应用。', clientIosCopy: '选择 iOS 客户端后，使用配置导入或二维码添加订阅。', clientLinuxCopy: 'Linux 桌面端可使用 Mihomo / Clash 客户端导入订阅。', clientRouterCopy: '路由器端请将订阅地址填入固件的 Clash / Mihomo 配置。', nodeStatus: '节点状态', nodeStatusCopy: '当前从 Provider 聚合的可用节点，按协议和地址快速确认。', nodeAvailable: '可用', nodeProtocol: '协议', nodeEndpoint: '地址', nodeNoData: '暂无可用节点', nodeNoDataCopy: 'Provider 注册并上报节点后，这里会显示节点状态。',
    providerDetails: 'Provider 状态与注册地址', nodeDetails: '当前可用节点', endpoint: '服务地址', protocol: '协议', lastSeen: '最近心跳', noNodes: '暂无可用节点。',
    qrCodeTitle: 'Shadowrocket 二维码', shadowrocketScan: '使用 Shadowrocket 扫描此二维码', downloadQr: '下载二维码', qrGenerating: '正在生成二维码…', qrLocalNote: '二维码在当前浏览器本地生成，不会上传订阅链接。',
    never: '从未', language: '语言',
    accountManagement: '账号与订阅链接', accountManagementCopy: '管理账号、分配流量，并为同一账号创建多个独立订阅链接。', createAccount: '创建账号', editAccount: '编辑账号', account: '账号', username: '用户名', displayName: '显示名称', links: '订阅链接', accountRecords: '{count} 个账号', noAccounts: '暂无账号。', accountCreated: '账号已创建', accountUpdated: '账号已更新', accountDeleted: '账号已删除', accountDisabled: '账号已停用', accountEnabled: '账号已启用', accountDetails: '账号详情', totalUsed: '已用流量', totalQuota: '总配额', linkCount: '链接数', openLinks: '查看链接', closeLinks: '收起链接', createLink: '创建订阅链接', linksFor: '{name} 的订阅链接', noLinks: '此账号暂无订阅链接。', linkCreated: '订阅链接已创建', linkUpdated: '订阅链接已更新', rotatePassword: '轮换密码', passwordRotated: '密码已轮换', linkDisabled: '订阅链接已停用', linkLabel: '链接名称', group: '订阅组', selectGroup: '选择订阅组', groupManagement: '订阅组', groupManagementCopy: '管理节点集合；固定 all 组始终包含当前全部可用节点。', createGroup: '创建订阅组', groupRecords: '{count} 个订阅组', noGroups: '暂无订阅组。', groupCreated: '订阅组已创建', groupUpdated: '订阅组已更新', groupDeleted: '订阅组已删除', protectedGroup: '固定组', nodeSelection: '节点选择', nodeLinkInput: '节点链接（每行一个）', currentNodes: '当前可用节点', customNodeLinks: '补充节点链接（每行一个）', allGroupNote: 'all 组自动包含全部当前节点，不能删除或手工修改。', save: '保存', delete: '删除', confirmDisable: '确定要停用吗？', confirmDelete: '确定要删除吗？历史用量会保留。', accountPlaceholder: 'customer-001', displayNamePlaceholder: '客户名称', groupPlaceholder: 'streaming', groupNamePlaceholder: '流媒体', linkLabelPlaceholder: '主订阅', chooseAccount: '选择账号', accountRequired: '请先选择账号', groupRequired: '请先选择订阅组', accountLinksHint: '同一账号可以分配多个链接，每条链接可以绑定不同订阅组。',
  },
  en: {
    workspace: 'Workspace', system: 'System', overview: 'Overview', nodes: 'Nodes', subscriptions: 'Subscriptions', accounts: 'Accounts', groups: 'Groups',
    tokens: 'Access tokens', traffic: 'Traffic', deployments: 'Deployments', settings: 'Settings', support: 'Support',
    administrator: 'Administrator', logout: 'Sign out', openNavigation: 'Open navigation', closeNavigation: 'Close navigation',
    switchLanguage: 'Switch language', chinese: '中文', english: 'EN', subscribeOnline: 'Subscribe online',
    subscribeUnavailable: 'Subscribe unavailable', refresh: 'Refresh', operations: 'Operations console',
    loginTitle: 'Operations console', loginCopy: 'Manage providers, subscription access, and usage policies in one place.',
    adminAccount: 'Admin account', adminPassword: 'Admin password', login: 'Sign in', credentialsNote: 'The login token lives only in this browser session (sessionStorage) and expires when the admin password changes', restoringSession: 'Restoring your session…',
    invalidCredentials: 'Could not verify admin credentials. Confirm the subscription service is running.', overviewCopy: 'Monitor providers, subscription access, and usage limits in real time.',
    manageTokens: 'Manage tokens', deployNode: 'Deploy node', activeSubscribers: 'Active subscribers', current: 'Current',
    subscriptionRequests: 'Subscription requests', persistedLocally: 'Persisted locally', trafficThisMonth: 'Traffic this month', measuredUsage: 'Measured usage',
    policyHealth: 'Policy health', withinLimits: 'Within limits', usageAllocation: 'Usage allocation',
    aggregateLimits: 'Aggregate limits across all subscription tokens', used: 'used', totalQuota: 'total quota', allocated: 'allocated',
    usageNote: 'Usage is persisted per token and updated on each subscription response.', providerRegistry: 'Provider registry',
    onlineRegistered: '{online} online · {total} registered', viewAll: 'View all', noProviders: 'No providers registered.',
    accessTokens: 'Access tokens', tokenUsageStatus: 'Token usage and policy status', tokenUsagePolicy: 'Configure a separate password, quota, and expiry for each subscriber.',
    createToken: 'Create token', allTokens: 'All tokens', tokenRecords: '{count} records · usage persisted locally',
    registeredProviders: 'Registered providers', heartbeatMonitored: '{count} records · heartbeat monitored',
    provider: 'Provider', status: 'Status', addresses: 'Addresses', nodeCount: 'Nodes', lastHeartbeat: 'Last heartbeat',
    online: 'Online', stale: 'Stale', disabled: 'Disabled', disable: 'Disable', noAdvertisedAddress: 'No advertised address',
    noProvidersYet: 'No providers have registered yet.', token: 'Token', usage: 'Usage', limit: 'Limit', requests: 'Requests',
    active: 'Active', passwordProtected: 'Password protected', noTokens: 'No tokens yet. Create the first scoped access token.',
    newPolicy: 'New policy', subscriptionPassword: 'Subscription password', quotaBytes: 'Quota (bytes)', expiresOptional: 'Expiry (optional)',
    quotaPeriod: 'Usage period', calendarMonth: 'Calendar month', creationMonth: 'Monthly from creation day', weekly: 'Weekly', daily: 'Daily', lifetime: 'No automatic reset',
    duration: 'Access duration', sevenDays: '7 days', oneMonth: '1 month', oneYear: '1 year', otherUid: 'External UID (optional)', nextReset: 'Next reset',
    createdAt: 'Created', renewal: 'Renewal', reset: 'Reset usage', renew7: 'Renew 7 days', renewMonth: 'Renew 1 month', renewYear: 'Renew 1 year',
    expiresAt: 'Expiry', expired: 'Expired', neverExpires: 'No expiry', retired: 'Retired (record kept)',
    profileName: 'Profile name', profileNamePlaceholder: 'vpn233', subscriptionLink: 'Clash subscription link', copyLink: 'Copy link', generateAndCopy: 'Generate & copy', copied: 'Copied', copyFailed: 'Copy failed. Copy it manually.', linkSecurityNote: 'This link contains the subscription password and is shown only during creation. Store it securely.', quickCopySecurityNote: 'Uses the token profile name; copying again does not rotate the password.', quickCopyDone: 'Subscription link copied',
    create: 'Create', cancel: 'Cancel', tokenPlaceholder: 'subscriber-001', passwordPlaceholder: 'Held separately by the subscriber',
    close: 'Close', tokenCreated: 'Token created', tokenDisabled: 'Token disabled', tokenDeleted: 'Token deleted', deleteToken: 'Delete token', confirmDeleteToken: 'Delete this token? Its subscription link will stop working and its usage history will be removed.', providerDisabled: 'Provider disabled',
    operationFailed: 'Operation failed: {message}', readFailed: 'Could not read service data: {message}',
    moduleReady: 'This module is connected to the unified console data layer. Refresh for the latest state.', moduleWorkspace: '{title} workspace',
    serviceSummary: 'The service is online with {nodes} nodes and {tokens} access policies.', refreshData: 'Refresh data',
    subscriptionWorkspaceCopy: 'Provide protected same-origin subscription entry points for Clash / Mihomo / sing-box / v2ray / Surge and more.', sameOriginDelivery: 'Same-origin subscription', sameOriginDeliveryCopy: 'Links use the current admin origin by default, so clients do not follow a stale host.', clashImportReady: 'Clash / Mihomo ready', clashImportReadyCopy: 'Returns YAML with the currently available Provider nodes.', subscriptionEndpoint: 'Subscription endpoint', providerNodes: 'Provider nodes', activeLinks: 'Active links', endpointPreview: 'The subscription password is never shown here.',
    trafficCopy: 'Review usage, requests, and reset cycles by token.', trafficUsed: 'Used traffic', trafficQuota: 'Total quota', trafficRequests: 'Subscription requests', trafficPolicies: 'Usage policies',
    trafficReset: 'Reset usage', trafficResetDone: 'Usage reset', trafficNoData: 'No usage records yet.', nextReset: 'Next reset',
    deploymentsCopy: 'Review providers, node counts, addresses, and recent heartbeats.', deploymentHealth: 'Deployment health', onlineProviders: 'Online providers', registeredNodes: 'Registered nodes', providerInventory: 'Provider inventory', nodeInventory: 'Node inventory',
    providerDetails: 'Provider status and advertised addresses', nodeDetails: 'Currently available nodes', endpoint: 'Endpoint', protocol: 'Protocol', lastSeen: 'Last heartbeat', noNodes: 'No available nodes.',
    qrCodeTitle: 'Shadowrocket QR code', shadowrocketScan: 'Scan this QR code with Shadowrocket', downloadQr: 'Download QR code', qrGenerating: 'Generating QR code…', qrLocalNote: 'The QR code is generated in this browser and the subscription link is not uploaded.',
    never: 'Never', language: 'Language',
    accountManagement: 'Accounts and links', accountManagementCopy: 'Manage accounts, allocate traffic, and create multiple scoped subscription links per account.', createAccount: 'Create account', editAccount: 'Edit account', account: 'Account', username: 'Username', displayName: 'Display name', links: 'Subscription links', accountRecords: '{count} accounts', noAccounts: 'No accounts yet.', accountCreated: 'Account created', accountUpdated: 'Account updated', accountDeleted: 'Account deleted', accountDisabled: 'Account disabled', accountEnabled: 'Account enabled', accountDetails: 'Account details', totalUsed: 'Used traffic', totalQuota: 'Total quota', linkCount: 'Links', openLinks: 'View links', closeLinks: 'Hide links', createLink: 'Create subscription link', linksFor: 'Subscription links for {name}', noLinks: 'This account has no subscription links.', linkCreated: 'Subscription link created', linkUpdated: 'Subscription link updated', rotatePassword: 'Rotate password', passwordRotated: 'Password rotated', linkDisabled: 'Subscription link disabled', linkLabel: 'Link label', group: 'Subscription group', selectGroup: 'Select group', groupManagement: 'Subscription groups', groupManagementCopy: 'Manage node collections; the fixed all group always contains all currently available nodes.', createGroup: 'Create group', groupRecords: '{count} groups', noGroups: 'No groups yet.', groupCreated: 'Group created', groupUpdated: 'Group updated', groupDeleted: 'Group deleted', protectedGroup: 'Protected', nodeSelection: 'Node selection', nodeLinkInput: 'Node links (one per line)', currentNodes: 'Current nodes', customNodeLinks: 'Additional node links (one per line)', allGroupNote: 'The all group automatically contains every current node and cannot be deleted or edited manually.', save: 'Save', delete: 'Delete', confirmDisable: 'Disable this item?', confirmDelete: 'Delete this item? Historical usage will be kept.', accountPlaceholder: 'customer-001', displayNamePlaceholder: 'Customer name', groupPlaceholder: 'streaming', groupNamePlaceholder: 'Streaming', linkLabelPlaceholder: 'Primary subscription', chooseAccount: 'Choose account', accountRequired: 'Choose an account first', groupRequired: 'Choose a group first', accountLinksHint: 'Each account can have multiple links, and each link can use a different subscription group.',
  },
}

// Copy for the format center, client import matrix, deploy commands, and privacy controls.
const extraCopy = {
  zh: {
    formatCenter: '订阅格式中心', formatCenterCopy: '选择访问令牌后，为 9 种订阅格式生成同源订阅链接；复制到客户端即可导入。',
    formatTokenSelect: '选择访问令牌', formatTokenPlaceholder: '选择一个已启用的令牌', formatEmpty: '暂无已启用的访问令牌，请先创建令牌。',
    formatTokenHint: '仅列出已启用的令牌；订阅链接与二维码均在当前浏览器本地生成，不会上传。',
    formatIssuing: '正在签发访问令牌…', formatIssueFailed: '无法签发访问令牌：{message}',
    formatNeedToken: '选择访问令牌后，这里会生成全部 9 种格式的订阅链接。',
    formatCopyLink: '复制链接', formatCopyCurl: '复制 curl', formatQr: '二维码', formatQrHide: '收起二维码',
    formatQrTitle: '扫描二维码导入订阅', formatQrLocalNote: '二维码在本地生成，订阅链接不会离开浏览器。',
    formatLinkCopied: '订阅链接已复制', formatCurlCopied: 'curl 命令已复制', formatLocalOnly: '本地生成',
    formatsLive: '实时格式', formatsFallback: '内置兜底', formatsFallbackNote: '未能读取 /api/v1/formats，当前展示内置兜底格式列表。',
    formatMatrix: '协议 × 格式能力矩阵', formatMatrixCopy: '数据来自 /api/v1/formats；勾选表示该订阅格式包含此协议。', formatMatrixEmpty: '暂无协议能力数据。',
    formatCount: '{count} 种格式', formatTokenIssued: '已为 {token} 签发访问令牌',
    clientSectionCopy: '按平台选择官方客户端，查看推荐订阅格式与导入步骤；GitHub 下载支持镜像前缀切换。',
    ghMirror: '下载镜像', ghMirrorDirect: '直连 GitHub', ghMirrorNote: '镜像仅用于加速 GitHub 下载，不会上传任何数据；选择保存在本机浏览器。',
    clientThirdParty: '客户端由各自开发方提供，本项目不包含其二进制；请核对官方发布来源后再安装。',
    clientRepo: '官方仓库', clientStore: '官方商店', clientRecommendedFormat: '推荐格式', clientCopySub: '去复制订阅链接',
    clientStepsClash: '复制 clash 订阅链接 → 打开客户端「配置 / 订阅」→ 粘贴链接 → 更新配置。',
    clientStepsSingbox: '复制 singbox 订阅链接 → 客户端添加远程配置 → 粘贴链接 → 更新。',
    clientStepsV2ray: '复制 v2ray 或 links 订阅链接 → 客户端「订阅设置」→ 添加订阅 → 更新。',
    clientStepsShadowrocket: '复制 v2ray 订阅链接 → Shadowrocket「添加配置」→ 从剪贴板导入即可。',
    clientStepsHiddify: '复制 v2ray 订阅链接 → Hiddify「新建配置」→ 从剪贴板导入 → 连接。',
    clientStepsSurge: '复制 surge 订阅链接 → Surge「配置」→ 通过 URL 下载配置。',
    clientStepsStash: '复制 stash 订阅链接 → Stash「配置」→ 添加远程配置。',
    clientStepsLoon: '复制 loon 订阅链接 → Loon「订阅」→ 添加订阅。',
    clientStepsQuantumultx: '复制 quantumultx 订阅链接 → Quantumult X「配置文件」→ 远程导入。',
    clientStepsSurfboard: '复制 surfboard 订阅链接 → Surfboard「配置」→ 添加配置。',
    clientStepsRouter: '复制 clash 订阅链接 → 在路由器 Mihomo / OpenWrt 插件中粘贴订阅地址。',
    deployCommands: '一键安装命令', deployCommandsCopy: '在服务器上以管理员 / root 执行；脚本会校验发布资产、安装服务并注册开机启动。',
    deployVersion: '当前版本', deployPsTitle: 'Windows PowerShell', deployShTitle: 'Linux / macOS shell', deployCopy: '复制命令',
    deployChecksum: '安装脚本会校验 SHA256SUMS，资产缺失或校验不匹配时中止安装。',
    deployService: 'Linux 注册 systemd 服务，Windows 注册计划任务（Scheduled Task）。',
    deployNote: '版本号来自后台 /api/status；raw 地址指向 GitHub 主仓库 main 分支，并会按所选镜像拼接前缀。',
    tokenShow: '显示令牌', tokenHide: '隐藏令牌', tokenMaskedHint: '令牌默认掩码显示，复制订阅链接时使用完整值。',
  },
  en: {
    formatCenter: 'Subscription format center', formatCenterCopy: 'Pick an access token to generate same-origin subscription links for all 9 formats. Paste one into your client to import.',
    formatTokenSelect: 'Select access token', formatTokenPlaceholder: 'Choose an enabled token', formatEmpty: 'No enabled access tokens yet. Create one first.',
    formatTokenHint: 'Only enabled tokens are listed. Subscription links and QR codes are generated locally and never uploaded.',
    formatIssuing: 'Issuing access token…', formatIssueFailed: 'Could not issue access token: {message}',
    formatNeedToken: 'Pick an access token to generate subscription links for all 9 formats.',
    formatCopyLink: 'Copy link', formatCopyCurl: 'Copy curl', formatQr: 'QR code', formatQrHide: 'Hide QR code',
    formatQrTitle: 'Scan to import the subscription', formatQrLocalNote: 'The QR code is generated locally and the subscription URL never leaves this browser.',
    formatLinkCopied: 'Subscription link copied', formatCurlCopied: 'curl command copied', formatLocalOnly: 'Generated locally',
    formatsLive: 'Live formats', formatsFallback: 'Built-in fallback', formatsFallbackNote: 'Could not read /api/v1/formats; showing the built-in fallback format list.',
    formatMatrix: 'Protocol × format capability matrix', formatMatrixCopy: 'Data from /api/v1/formats; a check mark means the format carries that protocol.', formatMatrixEmpty: 'No capability data available.',
    formatCount: '{count} formats', formatTokenIssued: 'Access token issued for {token}',
    clientSectionCopy: 'Choose an official client per platform, review its recommended format and import steps; GitHub downloads support mirror prefixes.',
    ghMirror: 'Download mirror', ghMirrorDirect: 'GitHub direct', ghMirrorNote: 'Mirrors only accelerate GitHub downloads and never receive data. The choice is saved in this browser.',
    clientThirdParty: 'Clients are provided by their own developers. This project does not bundle their binaries; verify the official release source before installing.',
    clientRepo: 'Official repo', clientStore: 'Official store', clientRecommendedFormat: 'Recommended format', clientCopySub: 'Get subscription link',
    clientStepsClash: 'Copy the clash subscription link, open the client profiles section, paste the URL, then update the profile.',
    clientStepsSingbox: 'Copy the singbox subscription link, add a remote profile in the client, paste the URL, then update it.',
    clientStepsV2ray: 'Copy the v2ray or links subscription link, add it under subscriptions in the client, then update.',
    clientStepsShadowrocket: 'Copy the v2ray subscription link, tap Add Profile in Shadowrocket, then import from clipboard.',
    clientStepsHiddify: 'Copy the v2ray subscription link, create a profile in Hiddify, import from clipboard, then connect.',
    clientStepsSurge: 'Copy the surge subscription link, open Surge profiles, then download the profile by URL.',
    clientStepsStash: 'Copy the stash subscription link, open Stash profiles, then add a remote profile.',
    clientStepsLoon: 'Copy the loon subscription link, open Loon subscriptions, then add the subscription.',
    clientStepsQuantumultx: 'Copy the quantumultx subscription link, open Quantumult X profiles, then import remotely.',
    clientStepsSurfboard: 'Copy the surfboard subscription link, open Surfboard profiles, then add the profile.',
    clientStepsRouter: 'Copy the clash subscription link, then paste the URL into the router Mihomo / OpenWrt add-on.',
    deployCommands: 'One-command install', deployCommandsCopy: 'Run as administrator / root on the server. The installer verifies release assets, installs the service, and registers it at boot.',
    deployVersion: 'Current version', deployPsTitle: 'Windows PowerShell', deployShTitle: 'Linux / macOS shell', deployCopy: 'Copy command',
    deployChecksum: 'The installer verifies SHA256SUMS and aborts when assets are missing or checksums do not match.',
    deployService: 'Installs a systemd service on Linux and a scheduled task on Windows.',
    deployNote: 'The version comes from /api/status; raw URLs point to the main branch of the GitHub repository and get the selected mirror prefix.',
    tokenShow: 'Show token', tokenHide: 'Hide token', tokenMaskedHint: 'Tokens are masked by default; copy actions use the full value.',
    // Backfill translations that were missing from the original English copy object.
    trafficToday: 'Traffic today', trafficRange: 'Range', trafficDays7: 'Last 7 days', trafficDays30: 'Last 30 days', trafficDays90: 'Last 90 days',
    trafficDailyTrend: 'Daily usage', trafficDailyTrendCopy: 'Daily records split by upload and download.', trafficDestinations: 'Traffic destinations', trafficDestinationsCopy: 'HTTP / SOCKS targets sorted by usage.',
    trafficDailyTable: 'Daily detail', trafficDomainTable: 'Domain detail', trafficUpload: 'Upload', trafficDownload: 'Download', trafficTotal: 'Total', trafficConnections: 'Connections',
    trafficDomain: 'Destination domain', trafficLastSeen: 'Last seen', trafficSourceClient: 'Source: vpn233-client', trafficSourceSubscription: 'Source: subscribe server',
    trafficSourceNote: 'Only subscription response sizes are recorded today; connect the client stats API to see real proxy traffic and domains.',
    trafficClientNote: 'Domains come from HTTP / SOCKS proxy target hosts; HTTPS CONNECT cannot expose URL paths, only hostnames.',
    trafficNoDomains: 'No domain traffic yet', trafficNoDomainsCopy: 'Connect vpn233-client to the stats API to start recording per-domain usage.',
    trafficNoDaily: 'No daily records yet', trafficNoDailyCopy: 'History is kept per day once subscription requests or proxy connections happen.',
    trafficSource: 'Stats source', trafficDataFresh: 'Observed', trafficDomainCount: 'Domains', trafficRequestCount: 'Requests', trafficBytes: 'Bytes',
    clientSection: 'Clients', clientSearch: 'Search clients…', clientDownload: 'Download', clientDocs: 'Docs', clientImport: 'Import',
    clientUnavailable: 'Download URL not configured', clientRecommended: 'Recommended', clientWindows: 'Windows', clientMacos: 'macOS', clientAndroid: 'Android',
    clientIos: 'iOS', clientLinux: 'Linux', clientRouter: 'Router', clientClashVerge: 'Clash Verge Rev', clientClashVergeCopy: 'Lightweight Tauri desktop client for Windows and macOS.',
    clientClashParty: 'Mihomo Party', clientClashPartyCopy: 'Desktop Clash client for system-proxy scenarios.', clientAndroidCopy: 'Import the subscription into your Android client.',
    clientIosCopy: 'Import the subscription with a profile URL or QR code.', clientLinuxCopy: 'Use a Mihomo / Clash client on Linux and import the subscription.', clientRouterCopy: 'Paste the subscription URL into the router Clash / Mihomo add-on.',
    nodeStatus: 'Node status', nodeStatusCopy: 'Currently available nodes aggregated from providers, grouped by protocol and address.', nodeAvailable: 'Available', nodeProtocol: 'Protocol',
    nodeEndpoint: 'Endpoint', nodeNoData: 'No available nodes yet', nodeNoDataCopy: 'Nodes appear here after a provider registers and reports them.',
  },
}
for (const language of Object.keys(extraCopy)) Object.assign(copy[language], extraCopy[language])

function useLanguage() {
  const [language, setLanguage] = useState(() => {
    const saved = globalThis.localStorage?.getItem(LANGUAGE_KEY)
    if (saved === 'zh' || saved === 'en') return saved
    return globalThis.navigator?.language?.toLowerCase().startsWith('zh') ? 'zh' : 'en'
  })
  const changeLanguage = (next) => { setLanguage(next); globalThis.localStorage?.setItem(LANGUAGE_KEY, next) }
  const t = (key, variables = {}) => String(copy[language][key] || key).replace(/\{(\w+)\}/g, (_, name) => variables[name] ?? '')
  return { language, changeLanguage, t }
}

function bytes(value = 0) {
  if (value < 1024) return `${value} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let next = value / 1024
  let index = 0
  while (next >= 1024 && index < units.length - 1) { next /= 1024; index += 1 }
  return `${next.toFixed(next >= 100 ? 0 : 1)} ${units[index]}`
}

function pct(used, limit) { return limit ? Math.min(100, Math.round((used / limit) * 100)) : 0 }
function formatDate(timestamp, language, fallback = '-') { return timestamp ? new Date(timestamp * 1000).toLocaleString(language === 'zh' ? 'zh-CN' : 'en-US') : fallback }
function periodLabel(period, t) { return ({ calendar_month: t('calendarMonth'), monthly_from_creation: t('creationMonth'), weekly: t('weekly'), daily: t('daily'), lifetime: t('lifetime') }[period] || period) }
function subscriptionOrigin() {
  if (CONFIGURED_SUBSCRIPTION_ORIGIN) return CONFIGURED_SUBSCRIPTION_ORIGIN.replace(/\/+$/, '')
  if (import.meta.env.DEV && API) return API.replace(/\/+$/, '')
  if (globalThis.location?.origin && globalThis.location.origin !== 'null') return globalThis.location.origin
  return API || DEV_API_BASE
}
function subscriptionUrl(token, password, filename = 'vpn233', accessToken) {
  const url = new URL('/sub/c', subscriptionOrigin())
  const params = { token, filename }
  if (accessToken) params.access_token = accessToken
  else if (password) params.password = password
  url.search = new URLSearchParams(params).toString()
  return url.toString()
}

// ---------------------------------------------------------------------------
// Session-scoped auth storage (JWT never touches persistent localStorage).
// The historical key was stored in localStorage; it is migrated once and purged.
// ---------------------------------------------------------------------------
const AUTH_TOKEN_KEY_LEGACY = AUTH_TOKEN_KEY

function readStoredAuthToken() {
  const session = globalThis.sessionStorage
  const legacy = globalThis.localStorage
  let token = session?.getItem(AUTH_TOKEN_KEY) || ''
  if (!token && legacy) {
    const migrated = legacy.getItem(AUTH_TOKEN_KEY_LEGACY) || ''
    if (migrated) {
      session?.setItem(AUTH_TOKEN_KEY, migrated)
      token = migrated
    }
  }
  legacy?.removeItem(AUTH_TOKEN_KEY_LEGACY)
  return token
}
function storeAuthToken(token) {
  globalThis.sessionStorage?.setItem(AUTH_TOKEN_KEY, token)
  globalThis.localStorage?.removeItem(AUTH_TOKEN_KEY_LEGACY)
}
function clearStoredAuthToken() {
  globalThis.sessionStorage?.removeItem(AUTH_TOKEN_KEY)
  globalThis.localStorage?.removeItem(AUTH_TOKEN_KEY_LEGACY)
}

// ---------------------------------------------------------------------------
// External links are assembled at runtime (see URL_* constants above) so the
// production bundle contains no external URL literals and the console never
// loads a third-party resource. Mirrors only prepend a prefix to GitHub URLs.
// ---------------------------------------------------------------------------
const GITHUB_HOST = 'github.com'
const RAW_GITHUB_HOST = 'raw.githubusercontent.com'
const VPN233_REPO = 'neko233-com/vpn233-all'
const GH_MIRROR_KEY = 'vpn233-gh-mirror'
function externalUrl(host, pathname = '') { return `${URL_SCHEME_HTTPS}${URL_SEP}${host}${pathname}` }
const DEPLOY_RAW_BASE = externalUrl(RAW_GITHUB_HOST, `/${VPN233_REPO}/main/`)
const GH_REPO_URL = repo => externalUrl(GITHUB_HOST, `/${repo}`)
const GH_RELEASES_URL = repo => externalUrl(GITHUB_HOST, `/${repo}/releases/latest`)
const GH_MIRRORS = [
  { id: 'direct', prefix: '', label: 'GitHub' },
  { id: 'ghproxy', prefix: externalUrl('ghproxy.net', '/'), label: 'ghproxy.net' },
  { id: 'gh-llkk', prefix: externalUrl('gh.llkk.cc', '/'), label: 'gh.llkk.cc' },
  { id: 'ghfast', prefix: externalUrl('ghfast.top', '/'), label: 'ghfast.top' },
  { id: 'gh-proxy', prefix: externalUrl('gh-proxy.com', '/'), label: 'gh-proxy.com' },
]
function readGhMirror() {
  const saved = globalThis.localStorage?.getItem(GH_MIRROR_KEY)
  return GH_MIRRORS.some(mirror => mirror.prefix === saved) ? (saved || '') : ''
}
function storeGhMirror(prefix) { globalThis.localStorage?.setItem(GH_MIRROR_KEY, prefix) }

// ---------------------------------------------------------------------------
// Subscription formats (/api/v1/formats with a built-in fallback)
// ---------------------------------------------------------------------------
const REQUIRED_FORMAT_IDS = ['clash', 'singbox', 'v2ray', 'links', 'surge', 'quantumultx', 'loon', 'surfboard', 'stash']
const URI_PROTOCOLS = ['vless', 'vmess', 'trojan', 'ss', 'ssr', 'hysteria2', 'tuic', 'anytls', 'socks5', 'http']
const SINGBOX_PROTOCOLS = URI_PROTOCOLS.filter(protocol => protocol !== 'ssr')
const PROTOCOL_ORDER = [...URI_PROTOCOLS, 'hysteria', 'wireguard', 'snell', 'ssh', 'mieru']
const PROTOCOL_LABELS = {
  ss: 'Shadowsocks', ssr: 'ShadowsocksR', vmess: 'VMess', vless: 'VLESS', trojan: 'Trojan',
  hysteria2: 'Hysteria2', hysteria: 'Hysteria', tuic: 'TUIC', wireguard: 'WireGuard',
  http: 'HTTP', socks5: 'SOCKS5', anytls: 'AnyTLS', snell: 'Snell', ssh: 'SSH', mieru: 'Mieru',
}
const FORMAT_DISPLAY_NAMES = {
  clash: 'Clash / Mihomo', singbox: 'sing-box', v2ray: 'v2rayN / v2rayNG', links: 'Raw Links', surge: 'Surge',
  quantumultx: 'Quantumult X', loon: 'Loon', surfboard: 'Surfboard', stash: 'Stash',
}
const FALLBACK_FORMATS = [
  { id: 'clash', name: 'Clash / Mihomo', content_type: 'text/yaml; charset=utf-8', extension: 'yaml', protocols: PROTOCOL_ORDER, recommended: true },
  { id: 'singbox', name: 'sing-box', content_type: 'application/json; charset=utf-8', extension: 'json', protocols: SINGBOX_PROTOCOLS },
  { id: 'v2ray', name: 'v2rayN / v2rayNG', content_type: 'text/plain; charset=utf-8', extension: 'txt', protocols: URI_PROTOCOLS },
  { id: 'links', name: 'Raw Links', content_type: 'text/plain; charset=utf-8', extension: 'txt', protocols: URI_PROTOCOLS },
  { id: 'surge', name: 'Surge', content_type: 'text/plain; charset=utf-8', extension: 'conf', protocols: ['ss', 'vmess', 'trojan', 'socks5', 'http'] },
  { id: 'quantumultx', name: 'Quantumult X', content_type: 'text/plain; charset=utf-8', extension: 'conf', protocols: ['ss', 'vmess', 'vless', 'trojan', 'socks5', 'http'] },
  { id: 'loon', name: 'Loon', content_type: 'text/plain; charset=utf-8', extension: 'conf', protocols: ['ss', 'vmess', 'vless', 'trojan', 'hysteria2', 'socks5', 'http'] },
  { id: 'surfboard', name: 'Surfboard', content_type: 'text/plain; charset=utf-8', extension: 'conf', protocols: ['ss', 'ssr', 'vmess', 'trojan', 'socks5', 'http'] },
  { id: 'stash', name: 'Stash', content_type: 'text/yaml; charset=utf-8', extension: 'yaml', protocols: SINGBOX_PROTOCOLS },
]

function protocolLabel(id) { return PROTOCOL_LABELS[id] || String(id).toUpperCase() }
function formatDisplayName(id) { return FORMAT_DISPLAY_NAMES[id] || id }

function normalizeFormat(raw, index = 0) {
  if (!raw || typeof raw !== 'object') return null
  const id = String(raw.id || raw.format || raw.name || `format-${index}`).trim().toLowerCase()
  if (!id) return null
  const protocols = (Array.isArray(raw.protocols) ? raw.protocols : [])
    .map(protocol => (typeof protocol === 'string' ? protocol : protocol?.id || protocol?.name || ''))
    .map(protocol => String(protocol).trim().toLowerCase())
    .filter(Boolean)
  return {
    id,
    name: String(raw.name || raw.label || id),
    contentType: String(raw.content_type || raw.contentType || ''),
    extension: String(raw.extension || '').replace(/^\./, ''),
    protocols: [...new Set(protocols)],
    recommended: Boolean(raw.recommended),
  }
}

function mergeFormats(rawFormats) {
  const live = (Array.isArray(rawFormats) ? rawFormats : []).map(normalizeFormat).filter(Boolean)
  const merged = new Map(live.map(format => [format.id, format]))
  for (const fallback of FALLBACK_FORMATS) {
    if (!merged.has(fallback.id)) merged.set(fallback.id, normalizeFormat(fallback))
  }
  const ordered = REQUIRED_FORMAT_IDS.map(id => merged.get(id)).filter(Boolean)
  for (const format of merged.values()) if (!REQUIRED_FORMAT_IDS.includes(format.id)) ordered.push(format)
  return ordered
}

function normalizeProtocolEntries(rawProtocols, formats) {
  const entries = new Map()
  const push = (id, label = '') => {
    const key = String(id || '').trim().toLowerCase()
    if (!key) return
    const existing = entries.get(key)
    entries.set(key, { id: key, label: label || existing?.label || PROTOCOL_LABELS[key] || key.toUpperCase() })
  }
  for (const protocol of Array.isArray(rawProtocols) ? rawProtocols : []) {
    if (typeof protocol === 'string') push(protocol)
    else if (protocol && typeof protocol === 'object') push(protocol.id || protocol.name, protocol.name || protocol.label)
  }
  if (!entries.size) for (const format of formats) for (const protocol of format.protocols) push(protocol)
  const rank = id => { const position = PROTOCOL_ORDER.indexOf(id); return position === -1 ? PROTOCOL_ORDER.length : position }
  return [...entries.values()].sort((left, right) => rank(left.id) - rank(right.id) || left.id.localeCompare(right.id))
}

function formatSubscriptionUrl(formatId, token, accessToken) {
  const url = new URL(`/sub/${encodeURIComponent(formatId)}`, subscriptionOrigin())
  url.search = new URLSearchParams({ token, access_token: accessToken, filename: `vpn233-${formatId}` }).toString()
  return url.toString()
}
function curlForFormat(format, url) {
  const filename = `vpn233-${format.id}.${format.extension || 'txt'}`
  return `curl -fL '${url}' -o '${filename}'`
}

// ---------------------------------------------------------------------------
// Shared privacy helpers
// ---------------------------------------------------------------------------
function maskToken(value = '') {
  const token = String(value)
  if (!token) return '-'
  if (token.length <= 12) return `${token.slice(0, 2)}${'•'.repeat(Math.max(1, token.length - 4))}${token.slice(-2)}`
  return `${token.slice(0, 6)}…${token.slice(-4)}`
}

async function copyText(value) {
  if (globalThis.navigator?.clipboard?.writeText) return globalThis.navigator.clipboard.writeText(value)
  const input = globalThis.document?.createElement('textarea')
  if (!input) throw new Error('clipboard unavailable')
  input.value = value
  input.style.position = 'fixed'
  input.style.opacity = '0'
  globalThis.document.body.appendChild(input)
  input.select()
  const copied = globalThis.document.execCommand?.('copy')
  input.remove()
  if (!copied) throw new Error('clipboard unavailable')
}

async function request(path, options = {}, auth) {
  const headers = new Headers(options.headers || {})
  if (auth?.token) headers.set('Authorization', `Bearer ${auth.token}`)
  else if (auth) headers.set('Authorization', `Basic ${globalThis.btoa(`${auth.user}:${auth.password}`)}`)
  if (options.body && !headers.has('Content-Type')) headers.set('Content-Type', 'application/json')
  const response = await fetch(`${API}${path}`, { ...options, headers })
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`)
  const text = await response.text()
  return text ? JSON.parse(text) : null
}

function LanguageToggle({ language, changeLanguage, t }) {
  return <div className="language-toggle" role="group" aria-label={t('language')} title={t('switchLanguage')}>
    <button className={language === 'zh' ? 'selected' : ''} onClick={() => changeLanguage('zh')}>{t('chinese')}</button>
    <button className={language === 'en' ? 'selected' : ''} onClick={() => changeLanguage('en')}>{t('english')}</button>
  </div>
}

function Login({ onLogin, language, changeLanguage, t }) {
  const [user, setUser] = useState(DEFAULT_USER)
  const [password, setPassword] = useState(DEFAULT_PASSWORD)
  const [error, setError] = useState('')
  const submit = async (event) => {
    event.preventDefault(); setError('')
    try {
      const data = await request('/api/auth/login', { method: 'POST', body: JSON.stringify({ username: user, password }) })
      onLogin({ user: data.user || user, token: data.token })
    } catch { setError(t('invalidCredentials')) }
  }
  return <main className="login-shell">
    <div className="login-language"><LanguageToggle language={language} changeLanguage={changeLanguage} t={t} /></div>
    <section className="login-panel">
      <div className="brand-mark"><span><Network size={18} /></span> VPN233</div>
      <p className="eyebrow">{t('operations')}</p>
      <h1>{t('loginTitle')}</h1>
      <p className="login-copy">{t('loginCopy')}</p>
      <form onSubmit={submit} className="login-form">
        <label>{t('adminAccount')}<input value={user} onChange={e => setUser(e.target.value)} autoComplete="username" /></label>
        <label>{t('adminPassword')}<input type="password" value={password} onChange={e => setPassword(e.target.value)} autoComplete="current-password" /></label>
        {error && <div className="error-line"><CircleAlert size={15} /> {error}</div>}
        <button className="primary-button" type="submit"><ShieldCheck size={17} /> {t('login')}</button>
      </form>
      <div className="login-note"><KeyRound size={14} /> {t('credentialsNote')}</div>
    </section>
  </main>
}

function App() {
  const [auth, setAuth] = useState(null)
  const [restoring, setRestoring] = useState(true)
  const { language, changeLanguage, t } = useLanguage()
  useEffect(() => {
    const token = readStoredAuthToken()
    if (!token) { setRestoring(false); return }
    const storedAuth = { user: DEFAULT_USER, token }
    request('/api/status', {}, storedAuth).then(() => setAuth(storedAuth)).catch(() => clearStoredAuthToken()).finally(() => setRestoring(false))
  }, [])
  const onLogin = nextAuth => { if (nextAuth.token) storeAuthToken(nextAuth.token); setAuth(nextAuth) }
  const onLogout = () => { clearStoredAuthToken(); setAuth(null) }
  if (restoring) return <main className="login-shell"><section className="login-panel session-panel"><div className="brand-mark"><span><Network size={18} /></span> VPN233</div><p className="eyebrow">{t('operations')}</p><h1>{t('restoringSession')}</h1></section></main>
  return auth
    ? <Dashboard auth={auth} onLogout={onLogout} language={language} changeLanguage={changeLanguage} t={t} />
    : <Login onLogin={onLogin} language={language} changeLanguage={changeLanguage} t={t} />
}

function Dashboard({ auth, onLogout, language, changeLanguage, t }) {
  const [section, setSection] = useState('overview')
  const [mobileNav, setMobileNav] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [status, setStatus] = useState(null)
  const [nodes, setNodes] = useState([])
  const [tokens, setTokens] = useState([])
  const [providers, setProviders] = useState([])
  const [accounts, setAccounts] = useState([])
  const [groups, setGroups] = useState([])
  const [traffic, setTraffic] = useState(null)
  const [trafficLoading, setTrafficLoading] = useState(false)
  const [ghMirror, setGhMirrorState] = useState(() => readGhMirror())
  const setGhMirror = prefix => { setGhMirrorState(prefix); storeGhMirror(prefix) }
  const [toast, setToast] = useState('')
  const load = async () => {
    setRefreshing(true)
    try {
      const [nextStatus, nextNodes, nextTokens, nextProviders, nextAccounts, nextGroups, nextTraffic] = await Promise.all([
        request('/api/status', {}, auth), request('/api/nodes', {}, auth), request('/api/admin/tokens', {}, auth), request('/api/admin/providers', {}, auth), request('/api/admin/accounts', {}, auth), request('/api/admin/groups', {}, auth), request('/api/admin/traffic?days=30', {}, auth),
      ])
      const rawTokens = nextTokens.tokens || []
      const normalizedTokens = Array.isArray(rawTokens) ? rawTokens : Object.entries(rawTokens).map(([token, record]) => ({ token, ...record }))
      setStatus(nextStatus); setNodes(nextNodes); setTokens(normalizedTokens); setProviders(nextProviders.providers || []); setAccounts(nextAccounts.accounts || []); setGroups(nextGroups.groups || []); setTraffic(nextTraffic)
    } catch (error) { setToast(t('readFailed', { message: error.message })) } finally { setRefreshing(false) }
  }
  const refreshTraffic = async days => {
    setTrafficLoading(true)
    try { setTraffic(await request(`/api/admin/traffic?days=${days}`, {}, auth)) } catch (error) { setToast(t('readFailed', { message: error.message })) } finally { setTrafficLoading(false) }
  }
  useEffect(() => { void load() }, [auth.user, auth.password, auth.token])
  useEffect(() => { if (!toast) return undefined; const id = setTimeout(() => setToast(''), 3000); return () => clearTimeout(id) }, [toast])
  const totalUsed = tokens.reduce((sum, token) => sum + token.used_bytes, 0)
  const totalLimit = tokens.reduce((sum, token) => sum + token.limit_bytes, 0)
  const totalRequests = tokens.reduce((sum, token) => sum + token.requests, 0)
  const activeTokens = tokens.filter(token => token.enabled).length
  const nav = [
    { id: 'overview', label: t('overview'), icon: CircleGauge }, { id: 'nodes', label: t('nodes'), icon: Server },
    { id: 'accounts', label: t('accounts'), icon: Users }, { id: 'groups', label: t('groups'), icon: Network }, { id: 'subscriptions', label: t('subscriptions'), icon: Cable }, { id: 'tokens', label: t('tokens'), icon: KeyRound },
    { id: 'traffic', label: t('traffic'), icon: Activity }, { id: 'deployments', label: t('deployments'), icon: Boxes },
  ]
  const currentLabel = nav.find(item => item.id === section)?.label || t('overview')
  const selectSection = (next) => { setSection(next); setMobileNav(false) }
  return <div className="app-shell">
    {mobileNav && <button className="nav-scrim" aria-label={t('closeNavigation')} onClick={() => setMobileNav(false)} />}
    <aside className={`sidebar ${mobileNav ? 'open' : ''}`}>
      <div className="brand"><span className="brand-symbol"><Network size={17} /></span><span>VPN233</span><span className="brand-version">v2</span></div>
      <div className="nav-block"><p className="nav-label">{t('workspace')}</p>{nav.map(({ id, label, icon: Icon }) => <button key={id} className={`nav-item ${section === id ? 'active' : ''}`} onClick={() => selectSection(id)}><Icon size={17} /><span>{label}</span>{id === 'tokens' && <span className="nav-count">{activeTokens}</span>}</button>)}</div>
      <div className="sidebar-bottom"><p className="nav-label">{t('system')}</p><div className="system-status"><span className={`pulse ${status?.ok ? 'online' : 'offline'}`} /> {status?.ok ? t('subscribeOnline') : t('subscribeUnavailable')}</div><div className="operator"><span className="avatar">R</span><span><strong>root</strong><small>{t('administrator')}</small></span><button className="icon-button" title={t('logout')} onClick={onLogout}><LogOut size={15} /></button></div></div>
    </aside>
    <div className="content-shell">
      <header className="topbar"><button className="mobile-menu icon-button" onClick={() => setMobileNav(!mobileNav)} title={mobileNav ? t('closeNavigation') : t('openNavigation')}><Menu size={19} /></button><div><span className="crumb-muted">{t('workspace')} / </span><strong>{currentLabel}</strong></div><div className="top-actions"><span className="service-state"><span className={`pulse ${status?.ok ? 'online' : 'offline'}`} /> {status?.ok ? t('subscribeOnline') : t('subscribeUnavailable')}</span><LanguageToggle language={language} changeLanguage={changeLanguage} t={t} /><button className="icon-button" title={t('refresh')} onClick={load}><RefreshCw className={refreshing ? 'spin' : ''} size={17} /></button><button className="avatar avatar-top" title={t('administrator')}>R</button></div></header>
       <main className="main-content">
         {section === 'overview' ? <Overview nodes={nodes} providers={providers} tokens={tokens} totalUsed={totalUsed} totalLimit={totalLimit} activeTokens={activeTokens} auth={auth} onChanged={load} onToast={setToast} onDeploy={() => setSection('deployments')} onTokens={() => setSection('tokens')} onProviders={() => setSection('nodes')} onImport={() => setSection('subscriptions')} ghMirror={ghMirror} onGhMirrorChange={setGhMirror} language={language} t={t} /> : section === 'nodes' ? <Providers providers={providers} auth={auth} onChanged={load} onToast={setToast} language={language} t={t} /> : section === 'accounts' ? <Accounts accounts={accounts} groups={groups} auth={auth} onChanged={load} onToast={setToast} language={language} t={t} /> : section === 'groups' ? <Groups groups={groups} nodes={nodes} auth={auth} onChanged={load} onToast={setToast} language={language} t={t} /> : section === 'subscriptions' ? <Subscriptions nodes={nodes} tokens={tokens} auth={auth} onChanged={load} onToast={setToast} onTokens={() => setSection('tokens')} language={language} t={t} /> : section === 'tokens' ? <Tokens tokens={tokens} auth={auth} onChanged={load} onToast={setToast} t={t} /> : section === 'traffic' ? <Traffic tokens={tokens} traffic={traffic} trafficLoading={trafficLoading} auth={auth} onChanged={load} onRangeChange={refreshTraffic} onToast={setToast} language={language} t={t} /> : section === 'deployments' ? <Deployments nodes={nodes} providers={providers} version={status?.version} ghMirror={ghMirror} onGhMirrorChange={setGhMirror} onRefresh={load} onToast={setToast} language={language} t={t} /> : <SectionPlaceholder title={currentLabel} nodes={nodes} tokens={tokens} onRefresh={load} t={t} />}
      </main>
    </div>
    {toast && <div className="toast"><CircleAlert size={16} /> {toast}<button className="toast-close" title={t('close')} onClick={() => setToast('')}><X size={14} /></button></div>}
  </div>
}

function PageTitle({ title, copyText, action }) { return <div className="page-heading"><div><p className="eyebrow">VPN233</p><h1>{title}</h1><p>{copyText}</p></div>{action}</div> }

function Overview({ nodes, providers, tokens, totalUsed, totalLimit, activeTokens, auth, onChanged, onToast, onDeploy, onTokens, onProviders, onImport, ghMirror, onGhMirrorChange, language, t }) {
  const totalRequests = tokens.reduce((sum, token) => sum + token.requests, 0)
  const trafficTotal = bytes(totalUsed)
  return <><PageTitle title={t('overview')} copyText={t('overviewCopy')} action={<div className="heading-actions"><button className="secondary-button" onClick={onTokens}><KeyRound size={16} /> {t('manageTokens')}</button><button className="primary-button" onClick={onDeploy}><Plus size={16} /> {t('deployNode')}</button></div>} />
    <div className="metric-grid"><Metric icon={Users} label={t('activeSubscribers')} value={activeTokens.toLocaleString()} delta={t('current')} positive /><Metric icon={Cable} label={t('subscriptionRequests')} value={totalRequests.toLocaleString()} delta={t('persistedLocally')} positive /><Metric icon={Activity} label={t('trafficThisMonth')} value={trafficTotal} delta={t('measuredUsage')} positive /><Metric icon={ShieldCheck} label={t('policyHealth')} value={`${activeTokens}/${tokens.length}`} delta={t('withinLimits')} positive /></div>
    <div className="overview-grid"><section className="panel chart-panel"><div className="panel-heading"><div><h2>{t('usageAllocation')}</h2><p>{t('aggregateLimits')}</p></div><span className="select-button">{t('current')}</span></div><div className="usage-summary"><div><strong>{trafficTotal}</strong><span>{t('used')}</span></div><div className="usage-total"><span>{bytes(totalLimit)} {t('totalQuota')}</span><div className="usage-bar large"><span style={{ width: `${pct(totalUsed, totalLimit)}%` }} /></div><small>{pct(totalUsed, totalLimit)}% {t('allocated')}</small></div></div><div className="usage-note"><Activity size={15} /> {t('usageNote')}</div></section><section className="panel nodes-panel"><div className="panel-heading"><div><h2>{t('providerRegistry')}</h2><p>{t('onlineRegistered', { online: providers.filter(provider => provider.online).length, total: providers.length })}</p></div><button className="link-button" onClick={onProviders}>{t('viewAll')} <ArrowUpRight size={14} /></button></div><div className="node-list">{providers.length ? providers.slice(0, 3).map(provider => <div className="node-row" key={provider.provider_id}><span className={`node-icon ${provider.online ? '' : 'muted'}`}><Server size={15} /></span><span className="node-main"><strong>{provider.name}</strong><small>{provider.version} · {provider.node_count} {t('nodes').toLowerCase()}</small></span><span className="node-latency">{provider.online ? t('online') : t('disabled')}<small><i className="status-dot" /> {provider.addresses?.[0]?.value || t('noAdvertisedAddress')}</small></span></div>) : <div className="empty">{t('noProviders')}</div>}</div></section></div>
     <section className="panel token-panel"><div className="panel-heading"><div><h2>{t('accessTokens')}</h2><p>{t('tokenUsageStatus')}</p><small className="security-hint">{t('quickCopySecurityNote')}</small></div><button className="link-button" onClick={onTokens}>{t('manageTokens')} <ArrowUpRight size={14} /></button></div><TokenTable tokens={tokens.slice(0, 4)} quickCopy auth={auth} onChanged={onChanged} onToast={onToast} language={language} t={t} /></section>
     <ClientDownloads onImport={onImport} ghMirror={ghMirror} onGhMirrorChange={onGhMirrorChange} t={t} />
     <NodeStatusGrid nodes={nodes} t={t} />
  </>
}

const CLIENT_PLATFORMS = [
  ['windows', 'clientWindows'], ['macos', 'clientMacos'], ['android', 'clientAndroid'],
  ['ios', 'clientIos'], ['linux', 'clientLinux'], ['router', 'clientRouter'],
]

const CLIENT_MATRIX = {
  windows: [
    { name: 'Clash Verge Rev', repo: 'clash-verge-rev/clash-verge-rev', format: 'clash', steps: 'clientStepsClash', recommended: true },
    { name: 'Mihomo Party', repo: 'mihomo-party-org/mihomo-party', format: 'clash', steps: 'clientStepsClash' },
    { name: 'v2rayN', repo: '2dust/v2rayN', format: 'v2ray', steps: 'clientStepsV2ray' },
    { name: 'sing-box', repo: 'SagerNet/sing-box', format: 'singbox', steps: 'clientStepsSingbox' },
    { name: 'Hiddify', repo: 'hiddify/hiddify-app', format: 'v2ray', steps: 'clientStepsHiddify' },
    { name: 'NekoRay', repo: 'Matsuridayo/nekoray', format: 'v2ray', steps: 'clientStepsV2ray' },
  ],
  macos: [
    { name: 'Clash Verge Rev', repo: 'clash-verge-rev/clash-verge-rev', format: 'clash', steps: 'clientStepsClash', recommended: true },
    { name: 'Mihomo Party', repo: 'mihomo-party-org/mihomo-party', format: 'clash', steps: 'clientStepsClash' },
    { name: 'sing-box', repo: 'SagerNet/sing-box', format: 'singbox', steps: 'clientStepsSingbox' },
    { name: 'Surge', store: externalUrl('nssurge.com', '/'), format: 'surge', steps: 'clientStepsSurge' },
    { name: 'Stash', store: externalUrl('apps.apple.com', '/app/stash/id1596063349'), format: 'stash', steps: 'clientStepsStash' },
    { name: 'Loon', store: externalUrl('apps.apple.com', '/app/loon/id1373567447'), format: 'loon', steps: 'clientStepsLoon' },
    { name: 'Hiddify', repo: 'hiddify/hiddify-app', format: 'v2ray', steps: 'clientStepsHiddify' },
  ],
  android: [
    { name: 'Clash Meta for Android', repo: 'MetaCubeX/ClashMetaForAndroid', format: 'clash', steps: 'clientStepsClash' },
    { name: 'v2rayNG', repo: '2dust/v2rayNG', format: 'v2ray', steps: 'clientStepsV2ray', recommended: true },
    { name: 'NekoBox', repo: 'MatsuriDayo/NekoBoxForAndroid', format: 'v2ray', steps: 'clientStepsV2ray' },
    { name: 'Hiddify', repo: 'hiddify/hiddify-app', format: 'v2ray', steps: 'clientStepsHiddify' },
    { name: 'Surfboard', store: externalUrl('getsurfboard.com', '/'), format: 'surfboard', steps: 'clientStepsSurfboard' },
  ],
  ios: [
    { name: 'Shadowrocket', store: externalUrl('apps.apple.com', '/app/shadowrocket/id932747118'), format: 'v2ray', steps: 'clientStepsShadowrocket', recommended: true },
    { name: 'Stash', store: externalUrl('apps.apple.com', '/app/stash/id1596063349'), format: 'stash', steps: 'clientStepsStash' },
    { name: 'Surge', store: externalUrl('nssurge.com', '/'), format: 'surge', steps: 'clientStepsSurge' },
    { name: 'Loon', store: externalUrl('apps.apple.com', '/app/loon/id1373567447'), format: 'loon', steps: 'clientStepsLoon' },
    { name: 'Quantumult X', store: externalUrl('apps.apple.com', '/app/quantumult-x/id1443988620'), format: 'quantumultx', steps: 'clientStepsQuantumultx' },
    { name: 'sing-box', repo: 'SagerNet/sing-box', format: 'singbox', steps: 'clientStepsSingbox' },
    { name: 'Hiddify', repo: 'hiddify/hiddify-app', format: 'v2ray', steps: 'clientStepsHiddify' },
  ],
  linux: [
    { name: 'Mihomo', repo: 'MetaCubeX/mihomo', format: 'clash', steps: 'clientStepsClash', recommended: true },
    { name: 'sing-box', repo: 'SagerNet/sing-box', format: 'singbox', steps: 'clientStepsSingbox' },
    { name: 'Hiddify', repo: 'hiddify/hiddify-app', format: 'v2ray', steps: 'clientStepsHiddify' },
  ],
  router: [
    { name: 'OpenWrt Mihomo', repo: 'morytyann/OpenWrt-mihomo', format: 'clash', steps: 'clientStepsRouter', recommended: true },
  ],
}

function ClientPlatformIcon({ platform }) {
  if (platform === 'android' || platform === 'ios') return <Smartphone size={18} />
  if (platform === 'router') return <Router size={18} />
  return <Monitor size={18} />
}

function ClientDownloads({ onImport, ghMirror, onGhMirrorChange, t }) {
  const [platform, setPlatform] = useState('windows')
  const [search, setSearch] = useState('')
  const mirror = GH_MIRRORS.find(item => item.prefix === ghMirror) || GH_MIRRORS[0]
  const mirrored = url => (mirror.prefix ? `${mirror.prefix}${url}` : url)
  const rows = (CLIENT_MATRIX[platform] || []).filter(item => item.name.toLowerCase().includes(search.trim().toLowerCase()))
  return <section className="panel client-panel">
    <div className="panel-heading client-heading">
      <div><h2>{t('clientSection')}</h2><p>{t('clientSectionCopy')}</p><small className="client-note">{t('clientThirdParty')}</small></div>
      <div className="client-toolbar">
        <label className="mirror-select"><Globe size={14} /><span>{t('ghMirror')}</span><select value={ghMirror} onChange={event => onGhMirrorChange(event.target.value)} aria-label={t('ghMirror')}>{GH_MIRRORS.map(item => <option key={item.id} value={item.prefix}>{item.prefix ? item.label : t('ghMirrorDirect')}</option>)}</select></label>
        <small className="mirror-note">{t('ghMirrorNote')}</small>
        <div className="client-search"><ScanLine size={15} /><input value={search} onChange={event => setSearch(event.target.value)} placeholder={t('clientSearch')} aria-label={t('clientSearch')} /></div>
      </div>
    </div>
    <div className="client-tabs" role="tablist">{CLIENT_PLATFORMS.map(([id, label]) => <button key={id} className={platform === id ? 'active' : ''} role="tab" aria-selected={platform === id} onClick={() => setPlatform(id)}>{t(label)}</button>)}</div>
    <div className="client-list">{rows.length ? rows.map(item => <div className="client-row" key={`${platform}-${item.name}`}><span className="client-logo"><ClientPlatformIcon platform={platform} /></span><div className="client-main"><strong>{item.name} {item.recommended && <span className="client-recommended">{t('clientRecommended')}</span>}</strong><small>{t(item.steps)}</small><div className="client-meta"><span>{t('clientRecommendedFormat')}: {formatDisplayName(item.format)}</span><span>{item.repo || t('clientStore')}</span>{mirror.prefix && item.repo && <span>{mirror.label}</span>}</div></div><div className="client-actions"><a className="table-action" href={item.repo ? mirrored(GH_RELEASES_URL(item.repo)) : item.store} target="_blank" rel="noopener noreferrer" aria-label={`${t('clientDownload')} ${item.name}`}><Download size={13} /> {t('clientDownload')}</a><a className="table-action" href={item.repo ? GH_REPO_URL(item.repo) : item.store} target="_blank" rel="noopener noreferrer" aria-label={`${item.repo ? t('clientRepo') : t('clientStore')} ${item.name}`}><ExternalLink size={13} /> {item.repo ? t('clientRepo') : t('clientStore')}</a><button className="secondary-button" onClick={onImport} aria-label={`${t('clientCopySub')} ${item.name}`}><Cable size={13} /> {t('clientCopySub')}</button></div></div>) : <div className="empty">{t('clientSearch')}</div>}</div>
  </section>
}

function NodeStatusGrid({ nodes, t }) {
  return <section className="panel node-status-panel"><div className="panel-heading"><div><h2>{t('nodeStatus')}</h2><p>{t('nodeStatusCopy')}</p></div><span className="select-button">{nodes.length} {t('nodes')}</span></div>{nodes.length ? <div className="node-status-grid">{nodes.map(node => <article className="node-status-card" key={`${node.name}-${node.server}-${node.port}`}><div className="node-status-card-head"><span className="node-icon"><Server size={16} /></span><span className="status-badge good"><i /> {t('nodeAvailable')}</span></div><strong>{node.name}</strong><small className="node-status-endpoint">{nodeEndpoint(node)}</small><div className="node-status-tags"><span>{node.protocol}</span><span>{t('nodeProtocol')}</span></div></article>)}</div> : <div className="node-empty"><Server size={20} /><strong>{t('nodeNoData')}</strong><small>{t('nodeNoDataCopy')}</small></div>}</section>
}

function Metric({ icon: Icon, label, value, delta, positive }) { return <section className="metric"><span className="metric-icon"><Icon size={17} /></span><div><p>{label}</p><strong>{value}</strong><span className={positive ? 'metric-delta positive' : 'metric-delta'}>{delta}</span></div></section> }

function Tokens({ tokens, auth, onChanged, onToast, language, t }) {
  const [open, setOpen] = useState(false)
  return <><PageTitle title={t('accessTokens')} copyText={t('tokenUsagePolicy')} action={<button className="primary-button" onClick={() => setOpen(true)}><Plus size={16} /> {t('createToken')}</button>} /><section className="panel token-panel full"><div className="token-toolbar"><div className="filter-button"><SlidersHorizontal size={15} /> {t('allTokens')} <ChevronDown size={14} /></div><span>{t('tokenRecords', { count: tokens.length })} · {t('tokenMaskedHint')}</span></div><TokenTable tokens={tokens} full auth={auth} onChanged={onChanged} onToast={onToast} language={language} t={t} /></section>{open && <CreateToken auth={auth} onClose={() => setOpen(false)} onCreated={() => { onChanged(); onToast(t('tokenCreated')) }} onToast={onToast} t={t} />}</>
}

function Subscriptions({ nodes, tokens, auth, onChanged, onToast, onTokens, language, t }) {
  const activeLinks = tokens.filter(token => token.enabled).length
  const origin = subscriptionOrigin()
  const [formats, setFormats] = useState(() => mergeFormats([]))
  const [protocols, setProtocols] = useState(() => normalizeProtocolEntries([], mergeFormats([])))
  const [formatsSource, setFormatsSource] = useState('fallback')
  useEffect(() => {
    let active = true
    request('/api/v1/formats', {}, auth).then(data => {
      if (!active) return
      const nextFormats = mergeFormats(data?.formats)
      setFormats(nextFormats)
      setProtocols(normalizeProtocolEntries(data?.protocols, nextFormats))
      setFormatsSource('live')
    }).catch(() => { if (active) setFormatsSource('fallback') })
    return () => { active = false }
  }, [auth.token])
  return <><PageTitle title={t('subscriptions')} copyText={t('subscriptionWorkspaceCopy')} action={<button className="primary-button" onClick={onTokens}><KeyRound size={16} /> {t('manageTokens')}</button>} /><div className="subscription-grid"><section className="panel subscription-panel"><div className="panel-heading"><div><h2>{t('sameOriginDelivery')}</h2><p>{t('sameOriginDeliveryCopy')}</p></div><span className="status-badge good"><i /> {t('online')}</span></div><div className="subscription-origin"><span>{t('subscriptionEndpoint')}</span><code>{origin}/sub/c?token=…&amp;password=…</code></div><div className="subscription-checklist"><div><Check size={15} /><span><strong>{t('clashImportReady')}</strong><small>{t('clashImportReadyCopy')}</small></span></div><div><Check size={15} /><span><strong>{t('providerNodes')}</strong><small>{nodes.length} {t('nodes').toLowerCase()} · {t('endpointPreview')}</small></span></div></div></section><section className="panel subscription-panel"><div className="panel-heading"><div><h2>{t('activeLinks')}</h2><p>{t('tokenUsageStatus')}</p></div><Cable size={18} className="subscription-icon" /></div><strong className="subscription-count">{activeLinks.toLocaleString()}</strong><span className="subscription-count-label">{t('active')}</span><div className="usage-bar large"><span style={{ width: `${tokens.length ? Math.round((activeLinks / tokens.length) * 100) : 0}%` }} /></div></section></div><section className="panel token-panel full"><div className="panel-heading"><div><h2>{t('subscriptionLink')}</h2><p>{t('quickCopySecurityNote')}</p></div><button className="secondary-button" onClick={onTokens}><SlidersHorizontal size={15} /> {t('accessTokens')}</button></div><TokenTable tokens={tokens} quickCopy auth={auth} onChanged={onChanged} onToast={onToast} language={language} t={t} /></section><FormatsCenter formats={formats} source={formatsSource} tokens={tokens} auth={auth} onToast={onToast} t={t} /><FormatsMatrix formats={formats} protocols={protocols} source={formatsSource} t={t} /></>
}

function FormatsCenter({ formats, source, tokens, auth, onToast, t }) {
  const enabledTokens = tokens.filter(token => token.enabled)
  const [selectedToken, setSelectedToken] = useState('')
  const [accessToken, setAccessToken] = useState('')
  const [issuing, setIssuing] = useState(false)
  const [qrFormat, setQrFormat] = useState('')
  const [qrDataUrl, setQrDataUrl] = useState('')
  useEffect(() => {
    if (selectedToken && !enabledTokens.some(token => token.token === selectedToken)) {
      setSelectedToken(''); setAccessToken(''); setQrFormat(''); setQrDataUrl('')
    }
  }, [tokens])
  const selectToken = async event => {
    const token = event.target.value
    setSelectedToken(token)
    setAccessToken('')
    setQrFormat('')
    setQrDataUrl('')
    if (!token) return
    setIssuing(true)
    try {
      const data = await request(`/api/admin/tokens/${encodeURIComponent(token)}/link`, { method: 'POST' }, auth)
      if (!data?.access_token) throw new Error('access_token missing')
      setAccessToken(data.access_token)
      onToast(t('formatTokenIssued', { token: maskToken(token) }))
    } catch (error) {
      setSelectedToken('')
      onToast(t('formatIssueFailed', { message: error.message }))
    }
    finally { setIssuing(false) }
  }
  const urlFor = format => formatSubscriptionUrl(format.id, selectedToken, accessToken)
  const copyLink = async format => {
    try { await copyText(urlFor(format)); onToast(t('formatLinkCopied')) } catch { onToast(t('copyFailed')) }
  }
  const copyCurl = async format => {
    try { await copyText(curlForFormat(format, urlFor(format))); onToast(t('formatCurlCopied')) } catch { onToast(t('copyFailed')) }
  }
  const toggleQr = async format => {
    if (qrFormat === format.id) { setQrFormat(''); setQrDataUrl(''); return }
    setQrFormat(format.id)
    setQrDataUrl('')
    try { setQrDataUrl(await QRCode.toDataURL(urlFor(format), { width: 220, margin: 2, errorCorrectionLevel: 'M' })) } catch { onToast(t('copyFailed')) }
  }
  const ready = Boolean(selectedToken && accessToken)
  return <section className="panel formats-panel"><div className="panel-heading formats-heading"><div><h2>{t('formatCenter')}</h2><p>{t('formatCenterCopy')}</p><small className="client-note">{t('formatTokenHint')}</small></div><div className="formats-toolbar"><span className={`status-badge ${source === 'live' ? 'good' : 'off'}`}><i /> {source === 'live' ? t('formatsLive') : t('formatsFallback')}</span><span className="select-button">{t('formatCount', { count: formats.length })}</span></div></div><div className="formats-token-row"><label className="format-token-label">{t('formatTokenSelect')}<select value={selectedToken} onChange={selectToken} disabled={issuing || !enabledTokens.length} aria-label={t('formatTokenSelect')}><option value="">{t('formatTokenPlaceholder')}</option>{enabledTokens.map(record => <option key={record.token} value={record.token}>{record.other_uid || record.label || `${record.token.slice(0, 8)}…`}</option>)}</select></label>{issuing && <span className="format-issuing"><RefreshCw size={14} className="spin" /> {t('formatIssuing')}</span>}{!enabledTokens.length && <span className="muted-text">{t('formatEmpty')}</span>}</div>{source === 'fallback' && <div className="formats-fallback-note"><CircleAlert size={14} /> {t('formatsFallbackNote')}</div>}{ready ? <div className="format-grid">{formats.map(format => <article className="format-card" key={format.id}><div className="format-card-head"><div><strong>{format.name}</strong><span className="format-id">{format.id}</span></div>{format.recommended && <span className="client-recommended">{t('clientRecommended')}</span>}</div><div className="format-meta"><span>{format.contentType || 'text/plain'}</span>{format.extension && <span>.{format.extension}</span>}<span>{t('formatLocalOnly')}</span></div><div className="format-protocols">{format.protocols.map(protocol => <span key={protocol}>{protocolLabel(protocol)}</span>)}</div><input className="format-url" readOnly value={urlFor(format)} aria-label={`${format.name} ${t('subscriptionLink')}`} onFocus={event => event.target.select()} /><div className="format-actions"><button type="button" className="table-action" onClick={() => copyLink(format)} aria-label={`${t('formatCopyLink')} ${format.name}`}><Copy size={12} /> {t('formatCopyLink')}</button><button type="button" className="table-action" onClick={() => copyCurl(format)} aria-label={`${t('formatCopyCurl')} ${format.name}`}><Terminal size={12} /> {t('formatCopyCurl')}</button><button type="button" className="table-action" onClick={() => toggleQr(format)} aria-label={`${t('formatQr')} ${format.name}`}><QrCode size={12} /> {qrFormat === format.id ? t('formatQrHide') : t('formatQr')}</button></div>{qrFormat === format.id && <div className="format-qr"><div className="format-qr-image">{qrDataUrl ? <img src={qrDataUrl} alt={`${format.name} ${t('formatQrTitle')}`} /> : <RefreshCw size={16} className="spin" />}</div><small>{t('formatQrLocalNote')}</small></div>}</article>)}</div> : <div className="format-empty"><QrCode size={20} /><small>{t('formatNeedToken')}</small></div>}</section>
}

function FormatsMatrix({ formats, protocols, source, t }) {
  return <section className="panel formats-panel formats-matrix-panel"><div className="panel-heading formats-heading"><div><h2>{t('formatMatrix')}</h2><p>{t('formatMatrixCopy')}</p></div><span className={`status-badge ${source === 'live' ? 'good' : 'off'}`}><i /> {source === 'live' ? t('formatsLive') : t('formatsFallback')}</span></div><div className="table-wrap matrix-wrap"><table className="format-matrix"><thead><tr><th scope="col">{t('protocol')}</th>{formats.map(format => <th key={format.id} scope="col">{format.name}</th>)}</tr></thead><tbody>{protocols.length ? protocols.map(protocol => <tr key={protocol.id}><th scope="row"><strong>{protocol.label}</strong><small className="protocol-id">{protocol.id}</small></th>{formats.map(format => <td key={format.id}>{format.protocols.includes(protocol.id) ? <Check size={14} className="matrix-check" aria-hidden="true" /> : <span className="matrix-missing" aria-hidden="true">·</span>}</td>)}</tr>) : <tr><td colSpan={formats.length + 1} className="empty">{t('formatMatrixEmpty')}</td></tr>}</tbody></table></div></section>
}

function Accounts({ accounts, groups, auth, onChanged, onToast, language, t }) {
  const [createOpen, setCreateOpen] = useState(false)
  const [expanded, setExpanded] = useState(null)
  const [linkAccount, setLinkAccount] = useState(null)
  const [editAccount, setEditAccount] = useState(null)
  return <><PageTitle title={t('accounts')} copyText={t('accountManagementCopy')} action={<button className="primary-button" onClick={() => setCreateOpen(true)}><Plus size={16} /> {t('createAccount')}</button>} /><section className="panel token-panel full"><div className="token-toolbar"><div className="filter-button"><Users size={15} /> {t('account')}</div><span>{t('accountRecords', { count: accounts.length })}</span></div><div className="table-wrap"><table className="accounts-table"><thead><tr><th>{t('account')}</th><th>{t('links')}</th><th>{t('totalUsed')}</th><th>{t('totalQuota')}</th><th>{t('requests')}</th><th>{t('status')}</th><th aria-label={t('status')} /></tr></thead><tbody>{accounts.length ? accounts.map(account => <AccountRow key={account.account_id} account={account} groups={groups} expanded={expanded === account.account_id} onToggle={() => setExpanded(expanded === account.account_id ? null : account.account_id)} onCreateLink={() => setLinkAccount(account)} onEdit={() => setEditAccount(account)} auth={auth} onChanged={onChanged} onToast={onToast} language={language} t={t} />) : <tr><td colSpan={7} className="empty">{t('noAccounts')}</td></tr>}</tbody></table></div></section>{createOpen && <CreateAccount auth={auth} onClose={() => setCreateOpen(false)} onCreated={() => { setCreateOpen(false); onChanged(); onToast(t('accountCreated')) }} onToast={onToast} t={t} />}{editAccount && <EditAccount account={editAccount} auth={auth} onClose={() => setEditAccount(null)} onCreated={() => { setEditAccount(null); onChanged(); onToast(t('accountUpdated')) }} onToast={onToast} t={t} />}{linkAccount && <CreateLink account={linkAccount} groups={groups} auth={auth} onClose={() => setLinkAccount(null)} onCreated={() => { onChanged(); onToast(t('linkCreated')) }} onToast={onToast} t={t} />}</>
}

function AccountRow({ account, groups, expanded, onToggle, onCreateLink, onEdit, auth, onChanged, onToast, language, t }) {
  const [links, setLinks] = useState([])
  const loadLinks = async () => {
    try { const data = await request(`/api/admin/accounts/${encodeURIComponent(account.account_id)}/links`, {}, auth); setLinks(data.links || []) } catch (error) { onToast(t('operationFailed', { message: error.message })) }
  }
  useEffect(() => { if (expanded) void loadLinks() }, [expanded])
  const toggle = () => { onToggle(); if (!expanded) void loadLinks() }
  const setEnabled = async enabled => {
    try { await request(`/api/admin/accounts/${encodeURIComponent(account.account_id)}`, { method: 'PATCH', body: JSON.stringify({ enabled }) }, auth); onChanged(); onToast(enabled ? t('accountEnabled') : t('accountDisabled')) } catch (error) { onToast(t('operationFailed', { message: error.message })) }
  }
  const remove = async () => {
    if (!globalThis.confirm?.(t('confirmDelete'))) return
    try {
      await request(`/api/admin/accounts/${encodeURIComponent(account.account_id)}`, { method: 'DELETE' }, auth)
      onChanged()
      onToast(t('accountDeleted'))
    } catch (error) { onToast(t('operationFailed', { message: error.message })) }
  }
  return <><tr><td><div className="token-cell"><span className="token-key"><Users size={14} /></span><span><strong>{account.display_name || account.username}</strong><small>{account.username} · {account.account_id}</small></span></div></td><td>{account.links_count}</td><td>{bytes(account.used_bytes)}</td><td>{bytes(account.limit_bytes)}</td><td>{account.requests.toLocaleString()}</td><td><span className={`status-badge ${account.enabled ? 'good' : 'off'}`}><i /> {account.enabled ? t('active') : t('disabled')}</span></td><td><div className="table-actions"><button className="table-action" onClick={onEdit}>{t('editAccount')}</button><button className="table-action" onClick={toggle}>{expanded ? t('closeLinks') : t('openLinks')}</button><button className="table-action" onClick={() => setEnabled(!account.enabled)} disabled={account.account_id === 'legacy'}>{account.enabled ? t('disable') : t('active')}</button><button className="table-action" onClick={remove} disabled={account.account_id === 'legacy'}>{t('delete')}</button></div></td></tr>{expanded && <tr><td colSpan={7} className="expanded-cell"><div className="expanded-head"><div><strong>{t('linksFor', { name: account.display_name || account.username })}</strong><small>{t('accountLinksHint')}</small></div><button className="primary-button" onClick={onCreateLink}><Plus size={15} /> {t('createLink')}</button></div>{links.length ? <div className="link-list">{links.map(link => <LinkRow key={link.token} link={link} groups={groups} auth={auth} onChanged={() => { loadLinks(); onChanged() }} onToast={onToast} language={language} t={t} />)}</div> : <div className="empty">{t('noLinks')}</div>}</td></tr>}</>
}

function LinkRow({ link, groups, auth, onChanged, onToast, language, t }) {
  const [generated, setGenerated] = useState('')
  const [qrDataUrl, setQrDataUrl] = useState('')
  const [rotating, setRotating] = useState(false)
  const copyStableLink = async () => {
    setRotating(true)
    try {
      const data = await request(`/api/admin/tokens/${encodeURIComponent(link.token)}/link`, { method: 'POST' }, auth)
      const url = subscriptionUrl(data.token || link.token, undefined, data.filename || 'vpn233', data.access_token)
      setGenerated(url)
      setQrDataUrl(await QRCode.toDataURL(url, { width: 240, margin: 2 }))
      onChanged()
      onToast(t('copied'))
    } catch (error) { onToast(t('operationFailed', { message: error.message })) } finally { setRotating(false) }
  }
  const disable = async () => { try { await request(`/api/admin/accounts/${encodeURIComponent(link.account_id)}/links/${encodeURIComponent(link.token)}`, { method: 'DELETE' }, auth); onChanged(); onToast(t('linkDisabled')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }
  const copy = async () => { try { await navigator.clipboard.writeText(generated); onToast(t('copied')) } catch { onToast(t('copyFailed')) } }
  return <div className="link-card"><div className="link-card-main"><span className="token-key"><Link2 size={14} /></span><div className="node-main"><strong>{link.label || maskToken(link.token)}</strong><small>{maskToken(link.token)} · {groups.find(group => group.group_id === link.group_id)?.name || link.group_id} · {bytes(link.used_bytes)} / {bytes(link.limit_bytes)}</small></div><span className={`status-badge ${link.enabled ? 'good' : 'off'}`}><i /> {link.enabled ? t('active') : t('disabled')}</span></div><div className="link-card-actions"><button className="table-action" onClick={copyStableLink} disabled={rotating || !link.enabled}><Copy size={12} className={rotating ? 'spin' : ''} /> {t('copyLink')}</button><button className="table-action" onClick={disable} disabled={!link.enabled}>{t('disable')}</button></div>{generated && <div className="generated-link-panel compact-generated"><div className="generated-link-row"><input className="generated-link-input" readOnly value={generated} aria-label={t('subscriptionLink')} /><button type="button" className="secondary-button" onClick={copy}><Copy size={14} /> {t('copyLink')}</button></div><div className="inline-qr">{qrDataUrl && <img src={qrDataUrl} alt={t('qrCodeTitle')} />}</div></div>}</div>
}

function CreateAccount({ auth, onClose, onCreated, onToast, t }) {
  const [form, setForm] = useState({ accountId: '', username: '', displayName: '', otherUid: '' }); const [saving, setSaving] = useState(false)
  const update = (field, value) => setForm(current => ({ ...current, [field]: value }))
  const submit = async event => { event.preventDefault(); setSaving(true); try { await request('/api/admin/accounts', { method: 'POST', body: JSON.stringify({ account_id: form.accountId || undefined, username: form.username, display_name: form.displayName || undefined, other_uid: form.otherUid || null }) }, auth); onCreated() } catch (error) { onToast(t('operationFailed', { message: error.message })) } finally { setSaving(false) } }
  return <Modal title={t('createAccount')} eyebrow={t('accountManagement')} onClose={onClose} t={t}><form onSubmit={submit} className="modal-form"><label>{t('username')}<input required value={form.username} placeholder={t('accountPlaceholder')} onChange={e => update('username', e.target.value)} /></label><label>{t('account')} ID<input value={form.accountId} placeholder={t('accountPlaceholder')} onChange={e => update('accountId', e.target.value)} /></label><label>{t('displayName')}<input value={form.displayName} placeholder={t('displayNamePlaceholder')} onChange={e => update('displayName', e.target.value)} /></label><label>{t('otherUid')}<input value={form.otherUid} onChange={e => update('otherUid', e.target.value)} /></label><div className="modal-foot"><button type="button" className="secondary-button" onClick={onClose}>{t('close')}</button><button className="primary-button" disabled={saving}><Check size={15} /> {t('save')}</button></div></form></Modal>
}

function EditAccount({ account, auth, onClose, onCreated, onToast, t }) {
  const [form, setForm] = useState({ displayName: account.display_name || '', otherUid: account.other_uid || '' }); const [saving, setSaving] = useState(false)
  const update = (field, value) => setForm(current => ({ ...current, [field]: value }))
  const submit = async event => { event.preventDefault(); setSaving(true); try { await request(`/api/admin/accounts/${encodeURIComponent(account.account_id)}`, { method: 'PATCH', body: JSON.stringify({ display_name: form.displayName, other_uid: form.otherUid || null }) }, auth); onCreated() } catch (error) { onToast(t('operationFailed', { message: error.message })) } finally { setSaving(false) } }
  return <Modal title={t('editAccount')} eyebrow={account.username} onClose={onClose} t={t}><form onSubmit={submit} className="modal-form"><label>{t('username')}<input disabled value={account.username} /></label><label>{t('account')} ID<input disabled value={account.account_id} /></label><label>{t('displayName')}<input value={form.displayName} onChange={e => update('displayName', e.target.value)} /></label><label>{t('otherUid')}<input value={form.otherUid} onChange={e => update('otherUid', e.target.value)} /></label><div className="modal-foot"><button type="button" className="secondary-button" onClick={onClose}>{t('close')}</button><button className="primary-button" disabled={saving}><Check size={15} /> {t('save')}</button></div></form></Modal>
}

function CreateLink({ account, groups, auth, onClose, onCreated, onToast, t }) {
  const [form, setForm] = useState({ token: '', password: '', label: '', groupId: groups.find(group => group.group_id === 'all')?.group_id || groups[0]?.group_id || 'all', limit: '107374182400', expires: '', quotaPeriod: 'monthly_from_creation', duration: 'one_month', otherUid: '' }); const [saving, setSaving] = useState(false); const [result, setResult] = useState(null); const [qrDataUrl, setQrDataUrl] = useState('')
  const update = (field, value) => setForm(current => ({ ...current, [field]: value }))
  const submit = async event => { event.preventDefault(); setSaving(true); try { const data = await request(`/api/admin/accounts/${encodeURIComponent(account.account_id)}/links`, { method: 'POST', body: JSON.stringify({ token: form.token, password: form.password, label: form.label || undefined, group_id: form.groupId, limit_bytes: Number(form.limit), expires_at: form.expires ? Math.floor(new Date(form.expires).getTime() / 1000) : null, quota_period: form.quotaPeriod, duration: form.expires ? undefined : form.duration, other_uid: form.otherUid || null }) }, auth); const url = subscriptionUrl(form.token.trim(), form.password, form.label.trim() || 'vpn233'); setResult({ url, password: data.password }); setQrDataUrl(await QRCode.toDataURL(url, { width: 260, margin: 2 })); onCreated() } catch (error) { onToast(t('operationFailed', { message: error.message })) } finally { setSaving(false) } }
  const copy = async () => { try { await navigator.clipboard.writeText(result.url.toString()); onToast(t('copied')) } catch { onToast(t('copyFailed')) } }
  return <Modal title={t('createLink')} eyebrow={t('linksFor', { name: account.display_name || account.username })} onClose={onClose} t={t}><form onSubmit={submit} className="modal-form"><label>{t('token')}<input required disabled={Boolean(result)} value={form.token} onChange={e => update('token', e.target.value)} placeholder="subscriber-001" /></label><label>{t('subscriptionPassword')}<input required disabled={Boolean(result)} type="password" value={form.password} onChange={e => update('password', e.target.value)} placeholder={t('passwordPlaceholder')} /></label><label>{t('linkLabel')}<input disabled={Boolean(result)} value={form.label} onChange={e => update('label', e.target.value)} placeholder={t('linkLabelPlaceholder')} /></label><label>{t('group')}<select disabled={Boolean(result)} value={form.groupId} onChange={e => update('groupId', e.target.value)}>{groups.map(group => <option key={group.group_id} value={group.group_id}>{group.name} · {group.node_count}</option>)}</select></label><label>{t('quotaBytes')}<input required disabled={Boolean(result)} type="number" min="1" value={form.limit} onChange={e => update('limit', e.target.value)} /></label><label>{t('quotaPeriod')}<select disabled={Boolean(result)} value={form.quotaPeriod} onChange={e => update('quotaPeriod', e.target.value)}><option value="calendar_month">{t('calendarMonth')}</option><option value="monthly_from_creation">{t('creationMonth')}</option><option value="weekly">{t('weekly')}</option><option value="daily">{t('daily')}</option><option value="lifetime">{t('lifetime')}</option></select></label><label>{t('duration')}<select disabled={Boolean(result)} value={form.duration} onChange={e => update('duration', e.target.value)}><option value="seven_days">{t('sevenDays')}</option><option value="one_month">{t('oneMonth')}</option><option value="one_year">{t('oneYear')}</option></select></label><label>{t('expiresOptional')}<input disabled={Boolean(result)} type="datetime-local" value={form.expires} onChange={e => update('expires', e.target.value)} /></label>{result && <><section className="generated-link-panel"><strong>{t('subscriptionLink')}</strong><p className="security-note">{t('linkSecurityNote')}</p><div className="generated-link-row"><input className="generated-link-input" readOnly value={result.url.toString()} /><button type="button" className="secondary-button" onClick={copy}><Copy size={14} /> {t('copyLink')}</button></div></section><section className="qr-panel"><div className="qr-copy"><strong>{t('qrCodeTitle')}</strong><p>{t('shadowrocketScan')}</p><small>{t('qrLocalNote')}</small></div><div className="qr-preview"><img src={qrDataUrl} alt={t('qrCodeTitle')} /></div></section></>}<div className="modal-foot"><button type="button" className="secondary-button" onClick={onClose}>{t('close')}</button><button type="submit" className="primary-button" disabled={saving || Boolean(result)}><Check size={15} /> {t('create')}</button></div></form></Modal>
}

function Modal({ title, eyebrow, onClose, t, children }) { return <div className="modal-backdrop" onMouseDown={onClose}><div className="modal" onMouseDown={event => event.stopPropagation()}><div className="modal-head"><div><p className="eyebrow">{eyebrow}</p><h2>{title}</h2></div><button className="icon-button" title={t('close')} onClick={onClose}><X size={18} /></button></div>{children}</div></div> }

function Groups({ groups, nodes, auth, onChanged, onToast, t }) {
  const [open, setOpen] = useState(false); const [editing, setEditing] = useState(null)
  return <><PageTitle title={t('groups')} copyText={t('groupManagementCopy')} action={<button className="primary-button" onClick={() => { setEditing(null); setOpen(true) }}><Plus size={16} /> {t('createGroup')}</button>} /><section className="panel token-panel full"><div className="token-toolbar"><div className="filter-button"><Network size={15} /> {t('groups')}</div><span>{t('groupRecords', { count: groups.length })}</span></div><div className="table-wrap"><table><thead><tr><th>{t('group')}</th><th>{t('nodeCount')}</th><th>{t('status')}</th><th aria-label={t('status')} /></tr></thead><tbody>{groups.length ? groups.map(group => <tr key={group.group_id}><td><div className="token-cell"><span className="token-key"><Network size={14} /></span><span><strong>{group.name}</strong><small>{group.group_id}</small></span></div></td><td>{group.node_count}</td><td>{group.protected ? <span className="status-badge"><i /> {t('protectedGroup')}</span> : <span className="status-badge good"><i /> {t('active')}</span>}</td><td><div className="table-actions"><button className="table-action" onClick={() => { setEditing(group); setOpen(true) }}>{t('save')}</button><button className="table-action" onClick={async () => { if (!confirm(t('confirmDelete'))) return; try { await request(`/api/admin/groups/${encodeURIComponent(group.group_id)}`, { method: 'DELETE' }, auth); onChanged(); onToast(t('groupDeleted')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }} disabled={group.protected}>{t('delete')}</button></div></td></tr>) : <tr><td colSpan={4} className="empty">{t('noGroups')}</td></tr>}</tbody></table></div></section>{open && <GroupModal group={editing} nodes={nodes} auth={auth} onClose={() => setOpen(false)} onChanged={() => { setOpen(false); onChanged(); onToast(editing ? t('groupUpdated') : t('groupCreated')) }} onToast={onToast} t={t} />}</>
}

function nodeEndpoint(node) { return node.endpoint || `${node.server}:${node.port}` }
function nodeSelection(node) { return node.payload || node.link }
function matchesNodeSelection(node, value) { return value === nodeSelection(node) || Boolean(node.link && value === node.link) }

function GroupModal({ group, nodes, auth, onClose, onChanged, onToast, t }) {
  const [form, setForm] = useState(() => {
    const saved = group?.node_links || []
    const known = value => nodes.some(node => matchesNodeSelection(node, value))
    return { groupId: group?.group_id || '', name: group?.name || '', nodeLinks: saved.filter(value => !known(value)).join('\n'), selectedLinks: saved.filter(known) }
  }); const [saving, setSaving] = useState(false)
  const update = (field, value) => setForm(current => ({ ...current, [field]: value }))
  const toggleNode = node => setForm(current => {
    const selected = current.selectedLinks.some(value => matchesNodeSelection(node, value))
    const remaining = current.selectedLinks.filter(value => !matchesNodeSelection(node, value))
    return { ...current, selectedLinks: selected ? remaining : [...remaining, nodeSelection(node)] }
  })
  const submit = async event => { event.preventDefault(); setSaving(true); try { const customLinks = form.nodeLinks.split(/\r?\n/).map(item => item.trim()).filter(Boolean); const links = [...new Set([...form.selectedLinks, ...customLinks])]; if (group) await request(`/api/admin/groups/${encodeURIComponent(group.group_id)}`, { method: 'PATCH', body: JSON.stringify({ name: form.name, node_links: group.protected ? [] : links }) }, auth); else await request('/api/admin/groups', { method: 'POST', body: JSON.stringify({ group_id: form.groupId || undefined, name: form.name, node_links: links }) }, auth); onChanged() } catch (error) { onToast(t('operationFailed', { message: error.message })) } finally { setSaving(false) } }
  return <Modal title={group ? group.name : t('createGroup')} eyebrow={t('groupManagement')} onClose={onClose} t={t}><form onSubmit={submit} className="modal-form"><label>{t('group')} ID<input required={!group} disabled={Boolean(group)} value={form.groupId} onChange={e => update('groupId', e.target.value)} placeholder={t('groupPlaceholder')} /></label><label>{t('displayName')}<input required value={form.name} disabled={Boolean(group?.protected)} onChange={e => update('name', e.target.value)} placeholder={t('groupNamePlaceholder')} /></label>{group?.protected ? <div className="wide-label all-group-note">{t('allGroupNote')}</div> : <><div className="wide-label"><span className="form-label">{t('currentNodes')}</span><div className="node-picker">{nodes.length ? nodes.map(node => <label className="node-option" key={nodeSelection(node)}><input type="checkbox" disabled={!nodeSelection(node)} checked={form.selectedLinks.some(value => matchesNodeSelection(node, value))} onChange={() => toggleNode(node)} /><span><strong>{node.name}</strong><small>{node.protocol} · {nodeEndpoint(node)}</small></span></label>) : <span className="muted-text">{t('noNodes')}</span>}</div></div><label className="wide-label">{t('customNodeLinks')}<textarea value={form.nodeLinks} onChange={e => update('nodeLinks', e.target.value)} rows={4} /></label></>}<div className="modal-foot"><button type="button" className="secondary-button" onClick={onClose}>{t('close')}</button><button className="primary-button" disabled={saving || Boolean(group?.protected)}><Check size={15} /> {t('save')}</button></div></form></Modal>
}

function formatSeen(timestamp, language) { return timestamp ? new Date(timestamp * 1000).toLocaleString(language === 'zh' ? 'zh-CN' : 'en-US') : null }

function Providers({ providers, auth, onChanged, onToast, language, t }) {
  const disable = async provider => {
    try { await request(`/api/admin/providers/${encodeURIComponent(provider.provider_id)}/disable`, { method: 'POST' }, auth); onChanged(); onToast(t('providerDisabled')) } catch (error) { onToast(t('operationFailed', { message: error.message })) }
  }
  return <><PageTitle title={t('providerRegistry')} copyText={t('onlineRegistered', { online: providers.filter(provider => provider.online).length, total: providers.length })} action={<button className="secondary-button" onClick={onChanged}><RefreshCw size={16} /> {t('refresh')}</button>} /><section className="panel token-panel full"><div className="token-toolbar"><div className="filter-button"><Network size={15} /> {t('registeredProviders')}</div><span>{t('heartbeatMonitored', { count: providers.length })}</span></div><div className="table-wrap"><table className="providers-table"><thead><tr><th>{t('provider')}</th><th>{t('status')}</th><th>{t('addresses')}</th><th>{t('nodeCount')}</th><th>{t('lastHeartbeat')}</th><th aria-label={t('status')} /></tr></thead><tbody>{providers.length ? providers.map(provider => <tr key={provider.provider_id}><td><div className="token-cell"><span className="token-key"><Server size={14} /></span><span><strong>{provider.name}</strong><small>{provider.provider_id} · v{provider.version}</small></span></div></td><td><span className={`status-badge ${provider.online ? 'good' : 'off'}`}><i /> {provider.online ? t('online') : provider.enabled ? t('stale') : t('disabled')}</span></td><td><div className="address-stack">{provider.addresses?.length ? provider.addresses.map(address => <span key={`${address.kind}-${address.value}`}><b>{address.kind}</b> {address.value}</span>) : <span className="muted-text">{t('noAdvertisedAddress')}</span>}</div></td><td>{provider.node_count}</td><td>{formatSeen(provider.last_seen, language)}</td><td><button className="table-action" onClick={() => disable(provider)} disabled={!provider.enabled}>{provider.enabled ? t('disable') : t('disabled')}</button></td></tr>) : <tr><td colSpan={6} className="empty">{t('noProvidersYet')}</td></tr>}</tbody></table></div></section></>
}

function Traffic({ tokens, traffic, trafficLoading, auth, onChanged, onRangeChange, onToast, language, t }) {
  const [rangeDays, setRangeDays] = useState(30)
  const used = tokens.reduce((sum, token) => sum + token.used_bytes, 0)
  const quota = tokens.reduce((sum, token) => sum + token.limit_bytes, 0)
  const fallbackRequests = tokens.reduce((sum, token) => sum + token.requests, 0)
  const summary = traffic?.summary || {}
  const daily = traffic?.daily || []
  const domains = traffic?.domains || []
  const today = daily[daily.length - 1]
  const chartDaily = daily.map(day => ({ ...day, label: day.date.slice(5) }))
  const chartDomains = domains.slice(0, 8).map(item => ({ ...item, label: item.domain.replace(/^\[IP\] /, '').slice(0, 22) }))
  const changeRange = next => { setRangeDays(next); onRangeChange(next) }
  const reset = async token => {
    try { await request(`/api/admin/tokens/${encodeURIComponent(token)}/reset`, { method: 'POST' }, auth); onChanged(); onToast(t('trafficResetDone')) } catch (error) { onToast(t('operationFailed', { message: error.message })) }
  }
  return <><PageTitle title={t('traffic')} copyText={t('trafficCopy')} action={<div className="traffic-actions"><div className="range-switch" aria-label={t('trafficRange')}>{[7, 30, 90].map(days => <button key={days} className={rangeDays === days ? 'selected' : ''} onClick={() => changeRange(days)}>{t(`trafficDays${days}`)}</button>)}</div><button className="secondary-button" onClick={onChanged}><RefreshCw className={trafficLoading ? 'spin' : ''} size={16} /> {t('refresh')}</button></div>} /><div className="metric-grid traffic-metrics"><Metric icon={Gauge} label={t('trafficToday')} value={bytes(today?.total_bytes || 0)} delta={today?.date || t('never')} positive /><Metric icon={Boxes} label={t('trafficTotal')} value={bytes(summary.total_bytes || used)} delta={`${summary.download_bytes ? bytes(summary.download_bytes) : bytes(used)} ${t('trafficDownload')}`} positive /><Metric icon={Activity} label={t('trafficRequestCount')} value={(summary.requests || fallbackRequests).toLocaleString()} delta={traffic?.available ? t('trafficSourceClient') : t('trafficSourceSubscription')} positive /><Metric icon={RotateCcw} label={t('trafficDomainCount')} value={(summary.domains ?? domains.length).toLocaleString()} delta={traffic?.available ? t('trafficClientNote') : t('trafficSourceNote')} positive /></div><div className="traffic-source-note"><CircleAlert size={15} /><span><strong>{traffic?.available ? t('trafficSourceClient') : t('trafficSourceSubscription')}</strong>{traffic?.note || t('trafficSourceNote')}</span><small>{traffic?.observed_at ? `${t('trafficDataFresh')} ${formatDate(traffic.observed_at, language)}` : ''}</small></div><div className="traffic-chart-grid"><section className="panel traffic-chart-panel"><div className="panel-heading"><div><h2>{t('trafficDailyTrend')}</h2><p>{t('trafficDailyTrendCopy')}</p></div><span className="select-button">{rangeDays} 天</span></div>{chartDaily.length ? <div className="traffic-chart"><ResponsiveContainer width="100%" height={290}><AreaChart data={chartDaily} margin={{ top: 10, right: 10, left: 0, bottom: 0 }}><defs><linearGradient id="trafficFill" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stopColor="#0071e3" stopOpacity={0.25} /><stop offset="100%" stopColor="#0071e3" stopOpacity={0.02} /></linearGradient></defs><CartesianGrid stroke="#f0f0f2" vertical={false} /><XAxis dataKey="label" tickLine={false} axisLine={false} tick={{ fill: '#86868b', fontSize: 10 }} /><YAxis tickLine={false} axisLine={false} tick={{ fill: '#86868b', fontSize: 10 }} tickFormatter={value => bytes(Number(value))} width={58} /><Tooltip contentStyle={{ border: '1px solid #e5e5e7', borderRadius: 10, boxShadow: '0 8px 20px #00000012', fontSize: 11 }} formatter={value => [bytes(Number(value)), t('trafficTotal')]} /><Area type="monotone" dataKey="total_bytes" stroke="#0071e3" strokeWidth={2.5} fill="url(#trafficFill)" name={t('trafficTotal')} /></AreaChart></ResponsiveContainer></div> : <TrafficEmpty icon={Activity} title={t('trafficNoDaily')} copy={t('trafficNoDailyCopy')} />}</section><section className="panel traffic-chart-panel"><div className="panel-heading"><div><h2>{t('trafficDestinations')}</h2><p>{t('trafficDestinationsCopy')}</p></div><span className="select-button">Top {Math.min(8, domains.length)}</span></div>{chartDomains.length ? <div className="traffic-chart"><ResponsiveContainer width="100%" height={290}><BarChart data={chartDomains} layout="vertical" margin={{ top: 4, right: 10, left: 2, bottom: 0 }}><CartesianGrid stroke="#f0f0f2" horizontal={false} /><XAxis type="number" hide /><YAxis type="category" dataKey="label" width={110} tickLine={false} axisLine={false} tick={{ fill: '#6e6e73', fontSize: 10 }} /><Tooltip contentStyle={{ border: '1px solid #e5e5e7', borderRadius: 10, boxShadow: '0 8px 20px #00000012', fontSize: 11 }} formatter={value => [bytes(Number(value)), t('trafficBytes')]} /><Bar dataKey="total_bytes" fill="#34c759" radius={[0, 5, 5, 0]} name={t('trafficTotal')} /></BarChart></ResponsiveContainer></div> : <TrafficEmpty icon={Network} title={t('trafficNoDomains')} copy={t('trafficNoDomainsCopy')} />}</section></div><div className="traffic-detail-grid"><section className="panel token-panel full traffic-detail-panel"><div className="panel-heading"><div><h2>{t('trafficDailyTable')}</h2><p>{t('trafficDailyTrendCopy')}</p></div></div><div className="table-wrap"><table className="traffic-detail-table"><thead><tr><th>{t('trafficDailyTable')}</th><th>{t('trafficUpload')}</th><th>{t('trafficDownload')}</th><th>{t('trafficTotal')}</th><th>{t('trafficRequestCount')}</th><th>{t('trafficConnections')}</th></tr></thead><tbody>{daily.length ? [...daily].reverse().map(day => <tr key={day.date}><td><strong>{day.date}</strong></td><td>{bytes(day.upload_bytes)}</td><td>{bytes(day.download_bytes)}</td><td><strong>{bytes(day.total_bytes)}</strong></td><td>{day.requests.toLocaleString()}</td><td>{day.connections.toLocaleString()}</td></tr>) : <tr><td colSpan={6} className="empty">{t('trafficNoDaily')}</td></tr>}</tbody></table></div></section><section className="panel token-panel full traffic-detail-panel"><div className="panel-heading"><div><h2>{t('trafficDomainTable')}</h2><p>{t('trafficClientNote')}</p></div></div><div className="table-wrap"><table className="traffic-detail-table"><thead><tr><th>{t('trafficDomain')}</th><th>{t('trafficTotal')}</th><th>{t('trafficUpload')}</th><th>{t('trafficDownload')}</th><th>{t('trafficRequestCount')}</th><th>{t('trafficLastSeen')}</th></tr></thead><tbody>{domains.length ? domains.map(domain => <tr key={domain.domain}><td><strong className="domain-name">{domain.domain}</strong></td><td><strong>{bytes(domain.total_bytes)}</strong></td><td>{bytes(domain.upload_bytes)}</td><td>{bytes(domain.download_bytes)}</td><td>{domain.requests.toLocaleString()}</td><td>{formatDate(domain.last_seen, language, t('never'))}</td></tr>) : <tr><td colSpan={6} className="empty">{t('trafficNoDomains')}</td></tr>}</tbody></table></div></section></div><section className="panel token-panel full traffic-panel"><div className="panel-heading"><div><h2>{t('trafficPolicies')}</h2><p>{t('tokenUsageStatus')}</p></div><span className="select-button">{t('current')}</span></div><div className="traffic-list">{tokens.length ? tokens.map(record => <div className="traffic-row" key={record.token}><div className="traffic-token"><span className="token-key"><KeyRound size={14} /></span><div><strong>{record.token}</strong><small>{record.other_uid || t('passwordProtected')}</small></div></div><div className="traffic-progress"><div className="usage-bar"><span style={{ width: `${pct(record.used_bytes, record.limit_bytes)}%` }} /></div><small>{bytes(record.used_bytes)} / {bytes(record.limit_bytes)}</small></div><div className="traffic-meta"><span>{record.requests.toLocaleString()} {t('requests')}</span><small>{t('nextReset')}: {formatDate(record.next_reset_at, language, t('never'))}</small></div><div className="traffic-status"><span className={`status-badge ${record.enabled ? 'good' : 'off'}`}><i /> {record.enabled ? t('active') : t('disabled')}</span><button className="table-action" onClick={() => reset(record.token)}>{t('trafficReset')}</button></div></div>) : <div className="empty">{t('trafficNoData')}</div>}</div></section></>
}

function TrafficEmpty({ icon: Icon, title, copy }) { return <div className="traffic-empty"><span><Icon size={20} /></span><strong>{title}</strong><small>{copy}</small></div> }

function DeployCommands({ version, ghMirror, onGhMirrorChange, onToast, t }) {
  const mirror = GH_MIRRORS.find(item => item.prefix === ghMirror) || GH_MIRRORS[0]
  const mirrorMode = mirror.prefix || 'off'
  const psCommand = [
    "$setup = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-install-' + [guid]::NewGuid().ToString('N') + '.ps1')",
    mirror.prefix ? `try { Invoke-WebRequest '${mirror.prefix}${DEPLOY_RAW_BASE}install.ps1' -OutFile $setup -TimeoutSec 30 -ErrorAction Stop }\ncatch { Invoke-WebRequest '${DEPLOY_RAW_BASE}install.ps1' -OutFile $setup -TimeoutSec 30 -ErrorAction Stop }` : `Invoke-WebRequest '${DEPLOY_RAW_BASE}install.ps1' -OutFile $setup -TimeoutSec 30 -ErrorAction Stop`,
    `powershell -NoProfile -ExecutionPolicy Bypass -File $setup -Release -Mirror '${mirrorMode}' -Yes`,
  ].filter(Boolean).join('\n')
  const shCommand = [
    'setup="$(mktemp)"',
    mirror.prefix ? `(curl -fL --connect-timeout 5 --max-time 30 '${mirror.prefix}${DEPLOY_RAW_BASE}install.sh' -o "$setup" || curl -fL --connect-timeout 5 --max-time 30 '${DEPLOY_RAW_BASE}install.sh' -o "$setup") &&` : `curl -fL --connect-timeout 5 --max-time 30 '${DEPLOY_RAW_BASE}install.sh' -o "$setup" &&`,
    `bash "$setup" --release --mirror '${mirrorMode}' --yes`,
  ].join('\n')
  const blocks = [{ id: 'ps', title: t('deployPsTitle'), command: psCommand }, { id: 'sh', title: t('deployShTitle'), command: shCommand }]
  const copy = async command => { try { await copyText(command); onToast(t('copied')) } catch { onToast(t('copyFailed')) } }
  return <section className="panel deploy-panel"><div className="panel-heading deploy-heading"><div><h2>{t('deployCommands')}</h2><p>{t('deployCommandsCopy')}</p></div><div className="deploy-head-meta"><span className="select-button">{t('deployVersion')} v{version || APP_VERSION}</span><label className="mirror-select"><Globe size={14} /><span>{t('ghMirror')}</span><select value={ghMirror} onChange={event => onGhMirrorChange(event.target.value)} aria-label={t('ghMirror')}>{GH_MIRRORS.map(item => <option key={item.id} value={item.prefix}>{item.prefix ? item.label : t('ghMirrorDirect')}</option>)}</select></label></div></div><div className="deploy-command-grid">{blocks.map(block => <div className="deploy-command" key={block.id}><div className="deploy-command-head"><strong>{block.title}</strong><button type="button" className="table-action" onClick={() => copy(block.command)} aria-label={`${t('deployCopy')} ${block.title}`}><Copy size={12} /> {t('deployCopy')}</button></div><pre><code>{block.command}</code></pre></div>)}</div><div className="deploy-notes"><div><ShieldCheck size={14} /> {t('deployChecksum')}</div><div><ServerCog size={14} /> {t('deployService')}</div><small>{t('deployNote')}</small></div></section>
}

function Deployments({ nodes, providers, version, ghMirror, onGhMirrorChange, onRefresh, onToast, language, t }) {
  const online = providers.filter(provider => provider.online).length
  const nodeTotal = providers.reduce((sum, provider) => sum + provider.node_count, 0)
  return <><PageTitle title={t('deployments')} copyText={t('deploymentsCopy')} action={<button className="secondary-button" onClick={onRefresh}><RefreshCw size={16} /> {t('refresh')}</button>} /><DeployCommands version={version} ghMirror={ghMirror} onGhMirrorChange={onGhMirrorChange} onToast={onToast} t={t} /><div className="metric-grid deployment-metrics"><Metric icon={ServerCog} label={t('onlineProviders')} value={`${online}/${providers.length}`} delta={t('deploymentHealth')} positive /><Metric icon={Server} label={t('registeredNodes')} value={nodeTotal.toLocaleString()} delta={t('providerInventory')} positive /><Metric icon={Link2} label={t('endpoint')} value={providers.filter(provider => provider.addresses?.length).length.toLocaleString()} delta={t('online')} positive /><Metric icon={Clock3} label={t('lastSeen')} value={providers.length ? formatDate(Math.max(...providers.map(provider => provider.last_seen || 0)), language, t('never')) : t('never')} delta={t('heartbeatMonitored', { count: providers.length })} positive /></div><section className="panel token-panel full deployment-panel"><div className="panel-heading"><div><h2>{t('providerInventory')}</h2><p>{t('providerDetails')}</p></div><span className="select-button">{online} {t('online')}</span></div><div className="deployment-grid"><div className="deployment-table"><table><thead><tr><th>{t('provider')}</th><th>{t('status')}</th><th>{t('addresses')}</th><th>{t('nodeCount')}</th><th>{t('lastSeen')}</th></tr></thead><tbody>{providers.length ? providers.map(provider => <tr key={provider.provider_id}><td><div className="token-cell"><span className="token-key"><ServerCog size={14} /></span><span><strong>{provider.name}</strong><small>{provider.provider_id} · v{provider.version}</small></span></div></td><td><span className={`status-badge ${provider.online ? 'good' : 'off'}`}><i /> {provider.online ? t('online') : provider.enabled ? t('stale') : t('disabled')}</span></td><td><div className="address-stack">{provider.addresses?.length ? provider.addresses.map(address => <span key={`${address.kind}-${address.value}`}><b>{address.kind}</b> {address.value}</span>) : <span className="muted-text">{t('noAdvertisedAddress')}</span>}</div></td><td>{provider.node_count}</td><td>{formatDate(provider.last_seen, language, t('never'))}</td></tr>) : <tr><td colSpan={5} className="empty">{t('noProvidersYet')}</td></tr>}</tbody></table></div><div className="deployment-nodes"><div className="panel-heading"><div><h2>{t('nodeInventory')}</h2><p>{t('nodeDetails')}</p></div><span className="select-button">{nodes.length}</span></div><div className="node-list compact-node-list">{nodes.length ? nodes.map(node => <div className="node-row" key={`${node.name}-${node.server}-${node.port}`}><span className="node-icon"><Server size={15} /></span><span className="node-main"><strong>{node.name}</strong><small>{nodeEndpoint(node)}</small></span><span className="node-latency">{node.protocol}<small>{t('online')}</small></span></div>) : <div className="empty">{t('noNodes')}</div>}</div></div></div></section></>
}

function TokenTable({ tokens, full, quickCopy, auth, onChanged, onToast, language, t }) { const hasActions = full || quickCopy; return <div className="table-wrap"><table><thead><tr><th>{t('token')}</th><th>{t('usage')}</th><th>{t('limit')}</th><th>{t('quotaPeriod')}</th><th>{t('expiresAt')}</th><th>{t('status')}</th>{hasActions && <th aria-label={t('status')} />}</tr></thead><tbody>{tokens.length ? tokens.map(record => <TokenRow key={record.token} record={record} full={full} showQuickCopy={quickCopy} auth={auth} onChanged={onChanged} onToast={onToast} language={language} t={t} />) : <tr><td colSpan={hasActions ? 7 : 6} className="empty">{t('noTokens')}</td></tr>}</tbody></table></div> }

function TokenRow({ record, full, showQuickCopy, auth, onChanged, onToast, language, t }) {
  const used = pct(record.used_bytes, record.limit_bytes)
  const [revealed, setRevealed] = useState(false)
  const [renewOpen, setRenewOpen] = useState(false)
  const [copying, setCopying] = useState(false)
  const [generated, setGenerated] = useState('')
  const disable = async () => { try { await request(`/api/admin/tokens/${encodeURIComponent(record.token)}`, { method: 'POST' }, auth); onChanged(); onToast(t('tokenDisabled')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }
  const remove = async () => {
    if (!globalThis.confirm?.(t('confirmDeleteToken'))) return
    try { await request(`/api/admin/tokens/${encodeURIComponent(record.token)}`, { method: 'DELETE' }, auth); onChanged(); onToast(t('tokenDeleted')) } catch (error) { onToast(t('operationFailed', { message: error.message })) }
  }
  const reset = async () => { try { await request(`/api/admin/tokens/${encodeURIComponent(record.token)}/reset`, { method: 'POST' }, auth); onChanged(); onToast(t('reset')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }
  const renew = async duration => { try { await request(`/api/admin/tokens/${encodeURIComponent(record.token)}/renew`, { method: 'POST', body: JSON.stringify({ duration }) }, auth); setRenewOpen(false); onChanged(); onToast(t('renewal')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }
  const generateAndCopy = async () => {
    setCopying(true)
    try {
      const data = await request(`/api/admin/tokens/${encodeURIComponent(record.token)}/link`, { method: 'POST' }, auth)
      const url = subscriptionUrl(data.token || record.token, undefined, data.filename || 'vpn233', data.access_token)
      setGenerated(url)
      if (globalThis.navigator?.clipboard?.writeText) await globalThis.navigator.clipboard.writeText(url)
      else {
        const input = globalThis.document?.createElement('textarea')
        if (!input) throw new Error('clipboard unavailable')
        input.value = url; input.style.position = 'fixed'; input.style.opacity = '0'; globalThis.document.body.appendChild(input); input.select()
        if (!globalThis.document.execCommand('copy')) throw new Error('clipboard unavailable')
        input.remove()
      }
      setGenerated(''); onChanged(); onToast(t('quickCopyDone'))
    } catch (error) { onToast(error.message.includes('clipboard') ? t('copyFailed') : t('operationFailed', { message: error.message })) }
    finally { setCopying(false) }
  }
  const expired = !record.enabled && record.disabled_reason === 'expired'
  return <><tr><td><div className="token-cell"><span className="token-key"><KeyRound size={14} /></span><span className="token-cell-main"><strong className="token-value">{revealed ? record.token : maskToken(record.token)}</strong><small>{record.other_uid || t('passwordProtected')}</small></span><button type="button" className="token-reveal" onClick={() => setRevealed(!revealed)} title={revealed ? t('tokenHide') : t('tokenShow')} aria-label={`${revealed ? t('tokenHide') : t('tokenShow')} ${maskToken(record.token)}`}>{revealed ? <EyeOff size={12} /> : <Eye size={12} />}</button></div></td><td><div className="usage-cell"><div className="usage-bar"><span style={{ width: `${used}%` }} /></div><small>{bytes(record.used_bytes)} / {bytes(record.limit_bytes)} · {formatDate(record.next_reset_at, language, t('never'))}</small></div></td><td>{bytes(record.limit_bytes)}</td><td><span className="period-cell">{periodLabel(record.quota_period, t)}</span></td><td><small>{record.expires_at ? formatDate(record.expires_at, language) : t('neverExpires')}</small></td><td><span className={`status-badge ${record.enabled ? 'good' : 'off'}`}><i /> {record.enabled ? t('active') : expired ? t('expired') : t('disabled')}</span></td>{(full || showQuickCopy) && <td><div className="table-actions">{(showQuickCopy || full) && <button className="table-action quick-copy-action" onClick={generateAndCopy} disabled={copying || !record.enabled} title={t('quickCopySecurityNote')}><Copy size={12} className={copying ? 'spin' : ''} /> {copying ? t('copyLink') : t('copyLink')}</button>}{full && <><button className="table-action" onClick={() => setRenewOpen(!renewOpen)}>{t('renewal')}</button><button className="table-action" onClick={reset}>{t('reset')}</button><button className="table-action" onClick={disable} disabled={!record.enabled}>{record.enabled ? t('disable') : t('disabled')}</button><button className="table-action danger-action" onClick={remove} title={t('deleteToken')}><Trash2 size={12} /> {t('delete')}</button>{renewOpen && <div className="renew-menu"><button onClick={() => renew('seven_days')}>{t('renew7')}</button><button onClick={() => renew('one_month')}>{t('renewMonth')}</button><button onClick={() => renew('one_year')}>{t('renewYear')}</button></div>}</>}</div></td>}</tr>{generated && <tr><td colSpan={7} className="quick-copy-recovery"><div className="security-note">{t('copyFailed')}</div><div className="generated-link-row"><input className="generated-link-input" readOnly value={generated} aria-label={t('subscriptionLink')} /><button type="button" className="secondary-button" onClick={async () => { try { await navigator.clipboard.writeText(generated); setGenerated(''); onToast(t('copied')) } catch { onToast(t('copyFailed')) } }}><Copy size={13} /> {t('copyLink')}</button></div></td></tr>}</>
}

function CreateToken({ auth, onClose, onCreated, onToast, t }) {
  const [form, setForm] = useState({ token: '', password: '', limit: '107374182400', expires: '', quotaPeriod: 'monthly_from_creation', duration: 'one_month', otherUid: '', profileName: 'vpn233' })
  const [subscriptionLink, setSubscriptionLink] = useState('')
  const [copied, setCopied] = useState(false)
  const [qrDataUrl, setQrDataUrl] = useState('')
  const [saving, setSaving] = useState(false)
  const update = (field, value) => setForm(current => ({ ...current, [field]: value }))
  const submit = async event => {
    event.preventDefault()
    setSaving(true)
    try {
      await request('/api/admin/tokens', { method: 'POST', body: JSON.stringify({ token: form.token, password: form.password, profileName: form.profileName.trim() || 'vpn233', limit_bytes: Number(form.limit), expires_at: form.expires ? Math.floor(new Date(form.expires).getTime() / 1000) : null, quota_period: form.quotaPeriod, duration: form.expires ? undefined : form.duration, other_uid: form.otherUid || null }) }, auth)
      const url = subscriptionUrl(form.token.trim(), form.password, form.profileName.trim() || 'vpn233')
      setSubscriptionLink(url)
      setCopied(false)
      setQrDataUrl('')
      onCreated()
    } catch (error) { onToast(t('operationFailed', { message: error.message })) }
    finally { setSaving(false) }
  }
  const copyLink = async () => {
    try {
      if (globalThis.navigator?.clipboard?.writeText) await globalThis.navigator.clipboard.writeText(subscriptionLink)
      else {
        const input = globalThis.document?.querySelector('.generated-link-input')
        input?.select()
        if (!globalThis.document?.execCommand?.('copy')) throw new Error('clipboard unavailable')
      }
      setCopied(true)
      onToast(t('copied'))
    } catch { setCopied(false); onToast(t('copyFailed')) }
  }
  useEffect(() => {
    if (!subscriptionLink) return undefined
    let active = true
    QRCode.toDataURL(subscriptionLink, { width: 280, margin: 2, errorCorrectionLevel: 'M' })
      .then(dataUrl => { if (active) setQrDataUrl(dataUrl) })
      .catch(() => { if (active) setQrDataUrl('') })
    return () => { active = false }
  }, [subscriptionLink])
  const qrFilename = `${(form.profileName.trim() || 'vpn233').replace(/[^a-z0-9_-]+/gi, '_')}-shadowrocket.png`
  return <div className="modal-backdrop" onMouseDown={onClose}><div className="modal" onMouseDown={event => event.stopPropagation()}><div className="modal-head"><div><p className="eyebrow">{t('newPolicy')}</p><h2>{t('createToken')}</h2></div><button className="icon-button" title={t('close')} onClick={onClose}><X size={18} /></button></div><form onSubmit={submit} className="modal-form"><label>{t('token')}<input required disabled={Boolean(subscriptionLink)} value={form.token} placeholder={t('tokenPlaceholder')} onChange={e => update('token', e.target.value)} /></label><label>{t('subscriptionPassword')}<input required disabled={Boolean(subscriptionLink)} type="password" value={form.password} placeholder={t('passwordPlaceholder')} onChange={e => update('password', e.target.value)} /></label><label>{t('profileName')}<input required disabled={Boolean(subscriptionLink)} value={form.profileName} placeholder={t('profileNamePlaceholder')} onChange={e => update('profileName', e.target.value)} /></label><label>{t('quotaBytes')}<input required disabled={Boolean(subscriptionLink)} type="number" min="1" value={form.limit} onChange={e => update('limit', e.target.value)} /></label><label>{t('quotaPeriod')}<select disabled={Boolean(subscriptionLink)} value={form.quotaPeriod} onChange={e => update('quotaPeriod', e.target.value)}><option value="calendar_month">{t('calendarMonth')}</option><option value="monthly_from_creation">{t('creationMonth')}</option><option value="weekly">{t('weekly')}</option><option value="daily">{t('daily')}</option><option value="lifetime">{t('lifetime')}</option></select></label><label>{t('duration')}<select disabled={Boolean(subscriptionLink)} value={form.duration} onChange={e => update('duration', e.target.value)}><option value="seven_days">{t('sevenDays')}</option><option value="one_month">{t('oneMonth')}</option><option value="one_year">{t('oneYear')}</option></select></label><label>{t('expiresOptional')}<input disabled={Boolean(subscriptionLink)} type="datetime-local" value={form.expires} onChange={e => update('expires', e.target.value)} /></label><label>{t('otherUid')}<input disabled={Boolean(subscriptionLink)} value={form.otherUid} placeholder="order_xxx / user_xxx" onChange={e => update('otherUid', e.target.value)} /></label>{subscriptionLink && <><section className="generated-link-panel"><div><strong>{t('subscriptionLink')}</strong><p className="security-note">{t('linkSecurityNote')}</p></div><div className="generated-link-row"><input className="generated-link-input" readOnly value={subscriptionLink} aria-label={t('subscriptionLink')} /><button type="button" className="secondary-button" onClick={copyLink}><Copy size={15} /> {copied ? t('copied') : t('copyLink')}</button></div></section><section className="qr-panel"><div className="qr-copy"><strong>{t('qrCodeTitle')}</strong><p>{t('shadowrocketScan')}</p><small>{t('qrLocalNote')}</small></div><div className="qr-preview">{qrDataUrl ? <img src={qrDataUrl} alt={t('qrCodeTitle')} /> : <span><QrCode size={28} /> {t('qrGenerating')}</span>}</div>{qrDataUrl && <a className="secondary-button qr-download" href={qrDataUrl} download={qrFilename}><Download size={15} /> {t('downloadQr')}</a>}</section></>}<div className="modal-foot"><button type="button" className="secondary-button" onClick={onClose}>{t('close')}</button><button type="submit" className="primary-button" disabled={saving || Boolean(subscriptionLink)}><Check size={16} /> {saving ? t('create') : t('create')}</button></div></form></div></div>
}

function SectionPlaceholder({ title, nodes, tokens, onRefresh, t }) { return <><PageTitle title={title} copyText={t('moduleReady')} action={<button className="secondary-button" onClick={onRefresh}><RefreshCw size={16} /> {t('refresh')}</button>} /><section className="empty-section"><div className="empty-orbit"><Network size={25} /></div><h2>{t('moduleWorkspace', { title })}</h2><p>{t('serviceSummary', { nodes: nodes.length, tokens: tokens.length })}</p><button className="primary-button" onClick={onRefresh}>{t('refreshData')}</button></section></> }

const root = globalThis.__vpn233Root || (globalThis.__vpn233Root = createRoot(document.getElementById('root')))
root.render(<App />)
