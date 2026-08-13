import React, { useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import QRCode from 'qrcode'
import {
  Activity, ArrowUpRight, Boxes, Cable, Check, ChevronDown, CircleAlert, Copy,
  CircleGauge, Clock3, Download, Gauge, KeyRound, LifeBuoy, Link2, LogOut, Menu,
  Network, Plus, QrCode, RefreshCw, RotateCcw, ScanLine, Server, ServerCog,
  Settings2, ShieldCheck, SlidersHorizontal, Users, X,
} from 'lucide-react'
import './styles.css'

const API = import.meta.env.VITE_API_BASE_URL || (import.meta.env.DEV ? 'http://127.0.0.1:23444' : '')
const DEFAULT_USER = 'root'
const DEFAULT_PASSWORD = 'root'
const LANGUAGE_KEY = 'vpn233-admin-language'
const PUBLIC_SUBSCRIPTION_ORIGIN = import.meta.env.VITE_PUBLIC_SUBSCRIPTION_ORIGIN ||
  (import.meta.env.DEV ? API : 'https://vsub.neko233.com')

const copy = {
  zh: {
    workspace: '工作区', system: '系统', overview: '总览', nodes: '节点', subscriptions: '订阅', accounts: '账号', groups: '订阅组',
    tokens: '访问令牌', traffic: '流量', deployments: '部署', settings: '设置', support: '支持',
    administrator: '管理员', logout: '退出登录', openNavigation: '打开导航', closeNavigation: '关闭导航',
    switchLanguage: '切换语言', chinese: '中文', english: 'EN', subscribeOnline: '订阅服务在线',
    subscribeUnavailable: '订阅服务不可用', refresh: '刷新', operations: '运营控制台',
    loginTitle: '运营控制台', loginCopy: '统一管理节点、订阅访问和用量策略。', adminAccount: '管理员账号',
    adminPassword: '管理员密码', login: '登录控制台', credentialsNote: '凭据只保存在当前浏览器内存中',
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
    profileName: '配置文件名', profileNamePlaceholder: 'vsub233', subscriptionLink: 'Clash 订阅链接', copyLink: '复制链接', generateAndCopy: '生成并复制', copied: '已复制', copyFailed: '复制失败，请手动复制', linkSecurityNote: '链接包含订阅密码，仅在本次创建流程中显示，请妥善保存。', quickCopySecurityNote: '点击后会轮换密码，旧链接立即失效；完整链接只写入当前剪贴板。', quickCopyDone: '订阅链接已生成并复制，旧链接已失效',
    create: '创建', cancel: '取消', tokenPlaceholder: 'subscriber-001', passwordPlaceholder: '由订阅者单独持有',
    close: '关闭', tokenCreated: '令牌已创建', tokenDisabled: '令牌已停用', providerDisabled: 'Provider 已停用',
    operationFailed: '操作失败：{message}', readFailed: '读取服务数据失败：{message}',
    moduleReady: '该模块已接入统一控制台数据层，选择刷新以获取最新状态。', moduleWorkspace: '{title} 工作区',
    serviceSummary: '服务端已在线，当前有 {nodes} 个节点和 {tokens} 个访问策略。', refreshData: '刷新数据',
    trafficCopy: '按令牌查看用量、请求和刷新周期。', trafficUsed: '已使用流量', trafficQuota: '总配额', trafficRequests: '订阅请求', trafficPolicies: '用量策略',
    trafficReset: '刷新用量', trafficResetDone: '用量已刷新', trafficNoData: '暂无用量记录。', nextReset: '下次刷新',
    deploymentsCopy: '查看 Provider、节点数量、地址和最近心跳。', deploymentHealth: '部署健康度', onlineProviders: '在线 Provider', registeredNodes: '已注册节点', providerInventory: 'Provider 清单', nodeInventory: '节点清单',
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
    adminAccount: 'Admin account', adminPassword: 'Admin password', login: 'Sign in', credentialsNote: 'Credentials stay in this browser session',
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
    profileName: 'Profile name', profileNamePlaceholder: 'vsub233', subscriptionLink: 'Clash subscription link', copyLink: 'Copy link', generateAndCopy: 'Generate & copy', copied: 'Copied', copyFailed: 'Copy failed. Copy it manually.', linkSecurityNote: 'This link contains the subscription password and is shown only during creation. Store it securely.', quickCopySecurityNote: 'This rotates the password, invalidates the old link, and writes the complete link only to the current clipboard.', quickCopyDone: 'Subscription link generated and copied; the old link is invalid',
    create: 'Create', cancel: 'Cancel', tokenPlaceholder: 'subscriber-001', passwordPlaceholder: 'Held separately by the subscriber',
    close: 'Close', tokenCreated: 'Token created', tokenDisabled: 'Token disabled', providerDisabled: 'Provider disabled',
    operationFailed: 'Operation failed: {message}', readFailed: 'Could not read service data: {message}',
    moduleReady: 'This module is connected to the unified console data layer. Refresh for the latest state.', moduleWorkspace: '{title} workspace',
    serviceSummary: 'The service is online with {nodes} nodes and {tokens} access policies.', refreshData: 'Refresh data',
    trafficCopy: 'Review usage, requests, and reset cycles by token.', trafficUsed: 'Used traffic', trafficQuota: 'Total quota', trafficRequests: 'Subscription requests', trafficPolicies: 'Usage policies',
    trafficReset: 'Reset usage', trafficResetDone: 'Usage reset', trafficNoData: 'No usage records yet.', nextReset: 'Next reset',
    deploymentsCopy: 'Review providers, node counts, addresses, and recent heartbeats.', deploymentHealth: 'Deployment health', onlineProviders: 'Online providers', registeredNodes: 'Registered nodes', providerInventory: 'Provider inventory', nodeInventory: 'Node inventory',
    providerDetails: 'Provider status and advertised addresses', nodeDetails: 'Currently available nodes', endpoint: 'Endpoint', protocol: 'Protocol', lastSeen: 'Last heartbeat', noNodes: 'No available nodes.',
    qrCodeTitle: 'Shadowrocket QR code', shadowrocketScan: 'Scan this QR code with Shadowrocket', downloadQr: 'Download QR code', qrGenerating: 'Generating QR code…', qrLocalNote: 'The QR code is generated in this browser and the subscription link is not uploaded.',
    never: 'Never', language: 'Language',
    accountManagement: 'Accounts and links', accountManagementCopy: 'Manage accounts, allocate traffic, and create multiple scoped subscription links per account.', createAccount: 'Create account', editAccount: 'Edit account', account: 'Account', username: 'Username', displayName: 'Display name', links: 'Subscription links', accountRecords: '{count} accounts', noAccounts: 'No accounts yet.', accountCreated: 'Account created', accountUpdated: 'Account updated', accountDeleted: 'Account deleted', accountDisabled: 'Account disabled', accountEnabled: 'Account enabled', accountDetails: 'Account details', totalUsed: 'Used traffic', totalQuota: 'Total quota', linkCount: 'Links', openLinks: 'View links', closeLinks: 'Hide links', createLink: 'Create subscription link', linksFor: 'Subscription links for {name}', noLinks: 'This account has no subscription links.', linkCreated: 'Subscription link created', linkUpdated: 'Subscription link updated', rotatePassword: 'Rotate password', passwordRotated: 'Password rotated', linkDisabled: 'Subscription link disabled', linkLabel: 'Link label', group: 'Subscription group', selectGroup: 'Select group', groupManagement: 'Subscription groups', groupManagementCopy: 'Manage node collections; the fixed all group always contains all currently available nodes.', createGroup: 'Create group', groupRecords: '{count} groups', noGroups: 'No groups yet.', groupCreated: 'Group created', groupUpdated: 'Group updated', groupDeleted: 'Group deleted', protectedGroup: 'Protected', nodeSelection: 'Node selection', nodeLinkInput: 'Node links (one per line)', currentNodes: 'Current nodes', customNodeLinks: 'Additional node links (one per line)', allGroupNote: 'The all group automatically contains every current node and cannot be deleted or edited manually.', save: 'Save', delete: 'Delete', confirmDisable: 'Disable this item?', confirmDelete: 'Delete this item? Historical usage will be kept.', accountPlaceholder: 'customer-001', displayNamePlaceholder: 'Customer name', groupPlaceholder: 'streaming', groupNamePlaceholder: 'Streaming', linkLabelPlaceholder: 'Primary subscription', chooseAccount: 'Choose account', accountRequired: 'Choose an account first', groupRequired: 'Choose a group first', accountLinksHint: 'Each account can have multiple links, and each link can use a different subscription group.',
  },
}

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

async function request(path, options = {}, auth) {
  const headers = new Headers(options.headers || {})
  if (auth) headers.set('Authorization', `Basic ${globalThis.btoa(`${auth.user}:${auth.password}`)}`)
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
    const auth = { user, password }
    try { await request('/api/status', {}, auth); onLogin(auth) } catch { setError(t('invalidCredentials')) }
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
  const { language, changeLanguage, t } = useLanguage()
  return auth
    ? <Dashboard auth={auth} onLogout={() => setAuth(null)} language={language} changeLanguage={changeLanguage} t={t} />
    : <Login onLogin={setAuth} language={language} changeLanguage={changeLanguage} t={t} />
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
  const [toast, setToast] = useState('')
  const load = async () => {
    setRefreshing(true)
    try {
      const [nextStatus, nextNodes, nextTokens, nextProviders, nextAccounts, nextGroups] = await Promise.all([
        request('/api/status', {}, auth), request('/api/nodes', {}, auth), request('/api/admin/tokens', {}, auth), request('/api/admin/providers', {}, auth), request('/api/admin/accounts', {}, auth), request('/api/admin/groups', {}, auth),
      ])
      const rawTokens = nextTokens.tokens || []
      const normalizedTokens = Array.isArray(rawTokens) ? rawTokens : Object.entries(rawTokens).map(([token, record]) => ({ token, ...record }))
      setStatus(nextStatus); setNodes(nextNodes); setTokens(normalizedTokens); setProviders(nextProviders.providers || []); setAccounts(nextAccounts.accounts || []); setGroups(nextGroups.groups || [])
    } catch (error) { setToast(t('readFailed', { message: error.message })) } finally { setRefreshing(false) }
  }
  useEffect(() => { void load() }, [auth.user, auth.password])
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
      <div className="sidebar-bottom"><p className="nav-label">{t('system')}</p><button className="nav-item"><Settings2 size={17} /> <span>{t('settings')}</span></button><button className="nav-item"><LifeBuoy size={17} /> <span>{t('support')}</span></button><div className="operator"><span className="avatar">R</span><span><strong>root</strong><small>{t('administrator')}</small></span><button className="icon-button" title={t('logout')} onClick={onLogout}><LogOut size={15} /></button></div></div>
    </aside>
    <div className="content-shell">
      <header className="topbar"><button className="mobile-menu icon-button" onClick={() => setMobileNav(!mobileNav)} title={mobileNav ? t('closeNavigation') : t('openNavigation')}><Menu size={19} /></button><div><span className="crumb-muted">{t('workspace')} / </span><strong>{currentLabel}</strong></div><div className="top-actions"><span className="service-state"><span className={`pulse ${status?.ok ? 'online' : 'offline'}`} /> {status?.ok ? t('subscribeOnline') : t('subscribeUnavailable')}</span><LanguageToggle language={language} changeLanguage={changeLanguage} t={t} /><button className="icon-button" title={t('refresh')} onClick={load}><RefreshCw className={refreshing ? 'spin' : ''} size={17} /></button><button className="avatar avatar-top" title={t('administrator')}>R</button></div></header>
       <main className="main-content">
         {section === 'overview' ? <Overview nodes={nodes} providers={providers} tokens={tokens} totalUsed={totalUsed} totalLimit={totalLimit} activeTokens={activeTokens} auth={auth} onChanged={load} onToast={setToast} onDeploy={() => setSection('deployments')} onTokens={() => setSection('tokens')} onProviders={() => setSection('nodes')} language={language} t={t} /> : section === 'nodes' ? <Providers providers={providers} auth={auth} onChanged={load} onToast={setToast} language={language} t={t} /> : section === 'accounts' ? <Accounts accounts={accounts} groups={groups} auth={auth} onChanged={load} onToast={setToast} language={language} t={t} /> : section === 'groups' ? <Groups groups={groups} nodes={nodes} auth={auth} onChanged={load} onToast={setToast} language={language} t={t} /> : section === 'tokens' ? <Tokens tokens={tokens} auth={auth} onChanged={load} onToast={setToast} t={t} /> : section === 'traffic' ? <Traffic tokens={tokens} auth={auth} onChanged={load} onToast={setToast} language={language} t={t} /> : section === 'deployments' ? <Deployments nodes={nodes} providers={providers} onRefresh={load} language={language} t={t} /> : <SectionPlaceholder title={currentLabel} nodes={nodes} tokens={tokens} onRefresh={load} t={t} />}
      </main>
    </div>
    {toast && <div className="toast"><CircleAlert size={16} /> {toast}<button className="toast-close" title={t('close')} onClick={() => setToast('')}><X size={14} /></button></div>}
  </div>
}

function PageTitle({ title, copyText, action }) { return <div className="page-heading"><div><p className="eyebrow">VPN233</p><h1>{title}</h1><p>{copyText}</p></div>{action}</div> }

function Overview({ providers, tokens, totalUsed, totalLimit, activeTokens, auth, onChanged, onToast, onDeploy, onTokens, onProviders, language, t }) {
  const totalRequests = tokens.reduce((sum, token) => sum + token.requests, 0)
  const trafficTotal = bytes(totalUsed)
  return <><PageTitle title={t('overview')} copyText={t('overviewCopy')} action={<div className="heading-actions"><button className="secondary-button" onClick={onTokens}><KeyRound size={16} /> {t('manageTokens')}</button><button className="primary-button" onClick={onDeploy}><Plus size={16} /> {t('deployNode')}</button></div>} />
    <div className="metric-grid"><Metric icon={Users} label={t('activeSubscribers')} value={activeTokens.toLocaleString()} delta={t('current')} positive /><Metric icon={Cable} label={t('subscriptionRequests')} value={totalRequests.toLocaleString()} delta={t('persistedLocally')} positive /><Metric icon={Activity} label={t('trafficThisMonth')} value={trafficTotal} delta={t('measuredUsage')} positive /><Metric icon={ShieldCheck} label={t('policyHealth')} value={`${activeTokens}/${tokens.length}`} delta={t('withinLimits')} positive /></div>
    <div className="overview-grid"><section className="panel chart-panel"><div className="panel-heading"><div><h2>{t('usageAllocation')}</h2><p>{t('aggregateLimits')}</p></div><span className="select-button">{t('current')}</span></div><div className="usage-summary"><div><strong>{trafficTotal}</strong><span>{t('used')}</span></div><div className="usage-total"><span>{bytes(totalLimit)} {t('totalQuota')}</span><div className="usage-bar large"><span style={{ width: `${pct(totalUsed, totalLimit)}%` }} /></div><small>{pct(totalUsed, totalLimit)}% {t('allocated')}</small></div></div><div className="usage-note"><Activity size={15} /> {t('usageNote')}</div></section><section className="panel nodes-panel"><div className="panel-heading"><div><h2>{t('providerRegistry')}</h2><p>{t('onlineRegistered', { online: providers.filter(provider => provider.online).length, total: providers.length })}</p></div><button className="link-button" onClick={onProviders}>{t('viewAll')} <ArrowUpRight size={14} /></button></div><div className="node-list">{providers.length ? providers.slice(0, 3).map(provider => <div className="node-row" key={provider.provider_id}><span className={`node-icon ${provider.online ? '' : 'muted'}`}><Server size={15} /></span><span className="node-main"><strong>{provider.name}</strong><small>{provider.version} · {provider.node_count} {t('nodes').toLowerCase()}</small></span><span className="node-latency">{provider.online ? t('online') : t('disabled')}<small><i className="status-dot" /> {provider.addresses?.[0]?.value || t('noAdvertisedAddress')}</small></span></div>) : <div className="empty">{t('noProviders')}</div>}</div></section></div>
     <section className="panel token-panel"><div className="panel-heading"><div><h2>{t('accessTokens')}</h2><p>{t('tokenUsageStatus')}</p><small className="security-hint">{t('quickCopySecurityNote')}</small></div><button className="link-button" onClick={onTokens}>{t('manageTokens')} <ArrowUpRight size={14} /></button></div><TokenTable tokens={tokens.slice(0, 4)} quickCopy auth={auth} onChanged={onChanged} onToast={onToast} language={language} t={t} /></section>
  </>
}

function Metric({ icon: Icon, label, value, delta, positive }) { return <section className="metric"><span className="metric-icon"><Icon size={17} /></span><div><p>{label}</p><strong>{value}</strong><span className={positive ? 'metric-delta positive' : 'metric-delta'}>{delta}</span></div></section> }

function Tokens({ tokens, auth, onChanged, onToast, language, t }) {
  const [open, setOpen] = useState(false)
  return <><PageTitle title={t('accessTokens')} copyText={t('tokenUsagePolicy')} action={<button className="primary-button" onClick={() => setOpen(true)}><Plus size={16} /> {t('createToken')}</button>} /><section className="panel token-panel full"><div className="token-toolbar"><div className="filter-button"><SlidersHorizontal size={15} /> {t('allTokens')} <ChevronDown size={14} /></div><span>{t('tokenRecords', { count: tokens.length })}</span></div><TokenTable tokens={tokens} full auth={auth} onChanged={onChanged} onToast={onToast} language={language} t={t} /></section>{open && <CreateToken auth={auth} onClose={() => setOpen(false)} onCreated={() => { onChanged(); onToast(t('tokenCreated')) }} onToast={onToast} t={t} />}</>
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
  const [open, setOpen] = useState(false)
  const loadLinks = async () => {
    try { const data = await request(`/api/admin/accounts/${encodeURIComponent(account.account_id)}/links`, {}, auth); setLinks(data.links || []) } catch (error) { onToast(t('operationFailed', { message: error.message })) }
  }
  useEffect(() => { if (expanded) void loadLinks() }, [expanded])
  const toggle = () => { onToggle(); setOpen(!open); if (!expanded) void loadLinks() }
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
  const buildUrl = password => { const url = new URL('/sub/c', PUBLIC_SUBSCRIPTION_ORIGIN); url.search = new URLSearchParams({ token: link.token, password, filename: link.label || 'vsub233' }).toString(); return url.toString() }
  const rotate = async () => {
    setRotating(true)
    try { const data = await request(`/api/admin/accounts/${encodeURIComponent(link.account_id)}/links/${encodeURIComponent(link.token)}/rotate`, { method: 'POST' }, auth); const url = buildUrl(data.password); setGenerated(url); setQrDataUrl(await QRCode.toDataURL(url, { width: 240, margin: 2 })); onChanged(); onToast(t('passwordRotated')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } finally { setRotating(false) }
  }
  const disable = async () => { try { await request(`/api/admin/accounts/${encodeURIComponent(link.account_id)}/links/${encodeURIComponent(link.token)}`, { method: 'DELETE' }, auth); onChanged(); onToast(t('linkDisabled')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }
  const copy = async () => { try { await navigator.clipboard.writeText(generated); onToast(t('copied')) } catch { onToast(t('copyFailed')) } }
  return <div className="link-card"><div className="link-card-main"><span className="token-key"><Link2 size={14} /></span><div className="node-main"><strong>{link.label || link.token}</strong><small>{link.token} · {groups.find(group => group.group_id === link.group_id)?.name || link.group_id} · {bytes(link.used_bytes)} / {bytes(link.limit_bytes)}</small></div><span className={`status-badge ${link.enabled ? 'good' : 'off'}`}><i /> {link.enabled ? t('active') : t('disabled')}</span></div><div className="link-card-actions"><button className="table-action" onClick={rotate} disabled={rotating}><RefreshCw size={12} className={rotating ? 'spin' : ''} /> {t('rotatePassword')}</button><button className="table-action" onClick={disable} disabled={!link.enabled}>{t('disable')}</button></div>{generated && <div className="generated-link-panel compact-generated"><div className="generated-link-row"><input className="generated-link-input" readOnly value={generated} aria-label={t('subscriptionLink')} /><button type="button" className="secondary-button" onClick={copy}><Copy size={14} /> {t('copyLink')}</button></div><div className="inline-qr">{qrDataUrl && <img src={qrDataUrl} alt={t('qrCodeTitle')} />}</div></div>}</div>
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
  const submit = async event => { event.preventDefault(); setSaving(true); try { const data = await request(`/api/admin/accounts/${encodeURIComponent(account.account_id)}/links`, { method: 'POST', body: JSON.stringify({ token: form.token, password: form.password, label: form.label || undefined, group_id: form.groupId, limit_bytes: Number(form.limit), expires_at: form.expires ? Math.floor(new Date(form.expires).getTime() / 1000) : null, quota_period: form.quotaPeriod, duration: form.expires ? undefined : form.duration, other_uid: form.otherUid || null }) }, auth); const url = new URL('/sub/c', PUBLIC_SUBSCRIPTION_ORIGIN); url.search = new URLSearchParams({ token: form.token.trim(), password: form.password, filename: form.label.trim() || 'vsub233' }).toString(); setResult({ url, password: data.password }); setQrDataUrl(await QRCode.toDataURL(url.toString(), { width: 260, margin: 2 })); onCreated() } catch (error) { onToast(t('operationFailed', { message: error.message })) } finally { setSaving(false) } }
  const copy = async () => { try { await navigator.clipboard.writeText(result.url.toString()); onToast(t('copied')) } catch { onToast(t('copyFailed')) } }
  return <Modal title={t('createLink')} eyebrow={t('linksFor', { name: account.display_name || account.username })} onClose={onClose} t={t}><form onSubmit={submit} className="modal-form"><label>{t('token')}<input required disabled={Boolean(result)} value={form.token} onChange={e => update('token', e.target.value)} placeholder="subscriber-001" /></label><label>{t('subscriptionPassword')}<input required disabled={Boolean(result)} type="password" value={form.password} onChange={e => update('password', e.target.value)} placeholder={t('passwordPlaceholder')} /></label><label>{t('linkLabel')}<input disabled={Boolean(result)} value={form.label} onChange={e => update('label', e.target.value)} placeholder={t('linkLabelPlaceholder')} /></label><label>{t('group')}<select disabled={Boolean(result)} value={form.groupId} onChange={e => update('groupId', e.target.value)}>{groups.map(group => <option key={group.group_id} value={group.group_id}>{group.name} · {group.node_count}</option>)}</select></label><label>{t('quotaBytes')}<input required disabled={Boolean(result)} type="number" min="1" value={form.limit} onChange={e => update('limit', e.target.value)} /></label><label>{t('quotaPeriod')}<select disabled={Boolean(result)} value={form.quotaPeriod} onChange={e => update('quotaPeriod', e.target.value)}><option value="calendar_month">{t('calendarMonth')}</option><option value="monthly_from_creation">{t('creationMonth')}</option><option value="weekly">{t('weekly')}</option><option value="daily">{t('daily')}</option><option value="lifetime">{t('lifetime')}</option></select></label><label>{t('duration')}<select disabled={Boolean(result)} value={form.duration} onChange={e => update('duration', e.target.value)}><option value="seven_days">{t('sevenDays')}</option><option value="one_month">{t('oneMonth')}</option><option value="one_year">{t('oneYear')}</option></select></label><label>{t('expiresOptional')}<input disabled={Boolean(result)} type="datetime-local" value={form.expires} onChange={e => update('expires', e.target.value)} /></label>{result && <><section className="generated-link-panel"><strong>{t('subscriptionLink')}</strong><p className="security-note">{t('linkSecurityNote')}</p><div className="generated-link-row"><input className="generated-link-input" readOnly value={result.url.toString()} /><button type="button" className="secondary-button" onClick={copy}><Copy size={14} /> {t('copyLink')}</button></div></section><section className="qr-panel"><div className="qr-copy"><strong>{t('qrCodeTitle')}</strong><p>{t('shadowrocketScan')}</p><small>{t('qrLocalNote')}</small></div><div className="qr-preview"><img src={qrDataUrl} alt={t('qrCodeTitle')} /></div></section></>}<div className="modal-foot"><button type="button" className="secondary-button" onClick={onClose}>{t('close')}</button><button type="submit" className="primary-button" disabled={saving || Boolean(result)}><Check size={15} /> {t('create')}</button></div></form></Modal>
}

function Modal({ title, eyebrow, onClose, t, children }) { return <div className="modal-backdrop" onMouseDown={onClose}><div className="modal" onMouseDown={event => event.stopPropagation()}><div className="modal-head"><div><p className="eyebrow">{eyebrow}</p><h2>{title}</h2></div><button className="icon-button" title={t('close')} onClick={onClose}><X size={18} /></button></div>{children}</div></div> }

function Groups({ groups, nodes, auth, onChanged, onToast, t }) {
  const [open, setOpen] = useState(false); const [editing, setEditing] = useState(null)
  return <><PageTitle title={t('groups')} copyText={t('groupManagementCopy')} action={<button className="primary-button" onClick={() => { setEditing(null); setOpen(true) }}><Plus size={16} /> {t('createGroup')}</button>} /><section className="panel token-panel full"><div className="token-toolbar"><div className="filter-button"><Network size={15} /> {t('groups')}</div><span>{t('groupRecords', { count: groups.length })}</span></div><div className="table-wrap"><table><thead><tr><th>{t('group')}</th><th>{t('nodeCount')}</th><th>{t('status')}</th><th aria-label={t('status')} /></tr></thead><tbody>{groups.length ? groups.map(group => <tr key={group.group_id}><td><div className="token-cell"><span className="token-key"><Network size={14} /></span><span><strong>{group.name}</strong><small>{group.group_id}</small></span></div></td><td>{group.node_count}</td><td>{group.protected ? <span className="status-badge"><i /> {t('protectedGroup')}</span> : <span className="status-badge good"><i /> {t('active')}</span>}</td><td><div className="table-actions"><button className="table-action" onClick={() => { setEditing(group); setOpen(true) }}>{t('save')}</button><button className="table-action" onClick={async () => { if (!confirm(t('confirmDelete'))) return; try { await request(`/api/admin/groups/${encodeURIComponent(group.group_id)}`, { method: 'DELETE' }, auth); onChanged(); onToast(t('groupDeleted')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }} disabled={group.protected}>{t('delete')}</button></div></td></tr>) : <tr><td colSpan={4} className="empty">{t('noGroups')}</td></tr>}</tbody></table></div></section>{open && <GroupModal group={editing} nodes={nodes} auth={auth} onClose={() => setOpen(false)} onChanged={() => { setOpen(false); onChanged(); onToast(editing ? t('groupUpdated') : t('groupCreated')) }} onToast={onToast} t={t} />}</>
}

function GroupModal({ group, nodes, auth, onClose, onChanged, onToast, t }) {
  const [form, setForm] = useState({ groupId: group?.group_id || '', name: group?.name || '', nodeLinks: group?.node_links?.join('\n') || '', selectedLinks: group?.node_links || [] }); const [saving, setSaving] = useState(false)
  const update = (field, value) => setForm(current => ({ ...current, [field]: value }))
  const toggleNode = link => update('selectedLinks', form.selectedLinks.includes(link) ? form.selectedLinks.filter(item => item !== link) : [...form.selectedLinks, link])
  const submit = async event => { event.preventDefault(); setSaving(true); try { const customLinks = form.nodeLinks.split(/\r?\n/).map(item => item.trim()).filter(Boolean); const links = [...new Set([...form.selectedLinks, ...customLinks])]; if (group) await request(`/api/admin/groups/${encodeURIComponent(group.group_id)}`, { method: 'PATCH', body: JSON.stringify({ name: form.name, node_links: group.protected ? [] : links }) }, auth); else await request('/api/admin/groups', { method: 'POST', body: JSON.stringify({ group_id: form.groupId || undefined, name: form.name, node_links: links }) }, auth); onChanged() } catch (error) { onToast(t('operationFailed', { message: error.message })) } finally { setSaving(false) } }
  return <Modal title={group ? group.name : t('createGroup')} eyebrow={t('groupManagement')} onClose={onClose} t={t}><form onSubmit={submit} className="modal-form"><label>{t('group')} ID<input required={!group} disabled={Boolean(group)} value={form.groupId} onChange={e => update('groupId', e.target.value)} placeholder={t('groupPlaceholder')} /></label><label>{t('displayName')}<input required value={form.name} disabled={Boolean(group?.protected)} onChange={e => update('name', e.target.value)} placeholder={t('groupNamePlaceholder')} /></label>{group?.protected ? <div className="wide-label all-group-note">{t('allGroupNote')}</div> : <><div className="wide-label"><span className="form-label">{t('currentNodes')}</span><div className="node-picker">{nodes.length ? nodes.map(node => <label className="node-option" key={node.link}><input type="checkbox" checked={form.selectedLinks.includes(node.link)} onChange={() => toggleNode(node.link)} /><span><strong>{node.name}</strong><small>{node.protocol} · {node.server}:{node.port}</small></span></label>) : <span className="muted-text">{t('noNodes')}</span>}</div></div><label className="wide-label">{t('customNodeLinks')}<textarea value={form.nodeLinks} onChange={e => update('nodeLinks', e.target.value)} rows={4} /></label></>}<div className="modal-foot"><button type="button" className="secondary-button" onClick={onClose}>{t('close')}</button><button className="primary-button" disabled={saving || Boolean(group?.protected)}><Check size={15} /> {t('save')}</button></div></form></Modal>
}

function formatSeen(timestamp, language) { return timestamp ? new Date(timestamp * 1000).toLocaleString(language === 'zh' ? 'zh-CN' : 'en-US') : null }

function Providers({ providers, auth, onChanged, onToast, language, t }) {
  const disable = async provider => {
    try { await request(`/api/admin/providers/${encodeURIComponent(provider.provider_id)}/disable`, { method: 'POST' }, auth); onChanged(); onToast(t('providerDisabled')) } catch (error) { onToast(t('operationFailed', { message: error.message })) }
  }
  return <><PageTitle title={t('providerRegistry')} copyText={t('onlineRegistered', { online: providers.filter(provider => provider.online).length, total: providers.length })} action={<button className="secondary-button" onClick={onChanged}><RefreshCw size={16} /> {t('refresh')}</button>} /><section className="panel token-panel full"><div className="token-toolbar"><div className="filter-button"><Network size={15} /> {t('registeredProviders')}</div><span>{t('heartbeatMonitored', { count: providers.length })}</span></div><div className="table-wrap"><table className="providers-table"><thead><tr><th>{t('provider')}</th><th>{t('status')}</th><th>{t('addresses')}</th><th>{t('nodeCount')}</th><th>{t('lastHeartbeat')}</th><th aria-label={t('status')} /></tr></thead><tbody>{providers.length ? providers.map(provider => <tr key={provider.provider_id}><td><div className="token-cell"><span className="token-key"><Server size={14} /></span><span><strong>{provider.name}</strong><small>{provider.provider_id} · v{provider.version}</small></span></div></td><td><span className={`status-badge ${provider.online ? 'good' : 'off'}`}><i /> {provider.online ? t('online') : provider.enabled ? t('stale') : t('disabled')}</span></td><td><div className="address-stack">{provider.addresses?.length ? provider.addresses.map(address => <span key={`${address.kind}-${address.value}`}><b>{address.kind}</b> {address.value}</span>) : <span className="muted-text">{t('noAdvertisedAddress')}</span>}</div></td><td>{provider.node_count}</td><td>{formatSeen(provider.last_seen, language)}</td><td><button className="table-action" onClick={() => disable(provider)} disabled={!provider.enabled}>{provider.enabled ? t('disable') : t('disabled')}</button></td></tr>) : <tr><td colSpan={6} className="empty">{t('noProvidersYet')}</td></tr>}</tbody></table></div></section></>
}

function Traffic({ tokens, auth, onChanged, onToast, language, t }) {
  const used = tokens.reduce((sum, token) => sum + token.used_bytes, 0)
  const quota = tokens.reduce((sum, token) => sum + token.limit_bytes, 0)
  const requests = tokens.reduce((sum, token) => sum + token.requests, 0)
  const reset = async token => {
    try { await request(`/api/admin/tokens/${encodeURIComponent(token)}/reset`, { method: 'POST' }, auth); onChanged(); onToast(t('trafficResetDone')) } catch (error) { onToast(t('operationFailed', { message: error.message })) }
  }
  return <><PageTitle title={t('traffic')} copyText={t('trafficCopy')} action={<button className="secondary-button" onClick={onChanged}><RefreshCw size={16} /> {t('refresh')}</button>} /><div className="metric-grid traffic-metrics"><Metric icon={Gauge} label={t('trafficUsed')} value={bytes(used)} delta={`${pct(used, quota)}%`} positive /><Metric icon={Boxes} label={t('trafficQuota')} value={bytes(quota)} delta={t('trafficPolicies')} positive /><Metric icon={Activity} label={t('trafficRequests')} value={requests.toLocaleString()} delta={t('measuredUsage')} positive /><Metric icon={RotateCcw} label={t('trafficPolicies')} value={tokens.length.toLocaleString()} delta={t('persistedLocally')} positive /></div><section className="panel token-panel full traffic-panel"><div className="panel-heading"><div><h2>{t('traffic')}</h2><p>{t('tokenUsageStatus')}</p></div><span className="select-button">{t('current')}</span></div><div className="traffic-list">{tokens.length ? tokens.map(record => <div className="traffic-row" key={record.token}><div className="traffic-token"><span className="token-key"><KeyRound size={14} /></span><div><strong>{record.token}</strong><small>{record.other_uid || t('passwordProtected')}</small></div></div><div className="traffic-progress"><div className="usage-bar"><span style={{ width: `${pct(record.used_bytes, record.limit_bytes)}%` }} /></div><small>{bytes(record.used_bytes)} / {bytes(record.limit_bytes)}</small></div><div className="traffic-meta"><span>{record.requests.toLocaleString()} {t('requests')}</span><small>{t('nextReset')}: {formatDate(record.next_reset_at, language, t('never'))}</small></div><div className="traffic-status"><span className={`status-badge ${record.enabled ? 'good' : 'off'}`}><i /> {record.enabled ? t('active') : t('disabled')}</span><button className="table-action" onClick={() => reset(record.token)}>{t('trafficReset')}</button></div></div>) : <div className="empty">{t('trafficNoData')}</div>}</div></section></>
}

function Deployments({ nodes, providers, onRefresh, language, t }) {
  const online = providers.filter(provider => provider.online).length
  const nodeTotal = providers.reduce((sum, provider) => sum + provider.node_count, 0)
  return <><PageTitle title={t('deployments')} copyText={t('deploymentsCopy')} action={<button className="secondary-button" onClick={onRefresh}><RefreshCw size={16} /> {t('refresh')}</button>} /><div className="metric-grid deployment-metrics"><Metric icon={ServerCog} label={t('onlineProviders')} value={`${online}/${providers.length}`} delta={t('deploymentHealth')} positive /><Metric icon={Server} label={t('registeredNodes')} value={nodeTotal.toLocaleString()} delta={t('providerInventory')} positive /><Metric icon={Link2} label={t('endpoint')} value={providers.filter(provider => provider.addresses?.length).length.toLocaleString()} delta={t('online')} positive /><Metric icon={Clock3} label={t('lastSeen')} value={providers.length ? formatDate(Math.max(...providers.map(provider => provider.last_seen || 0)), language, t('never')) : t('never')} delta={t('heartbeatMonitored', { count: providers.length })} positive /></div><section className="panel token-panel full deployment-panel"><div className="panel-heading"><div><h2>{t('providerInventory')}</h2><p>{t('providerDetails')}</p></div><span className="select-button">{online} {t('online')}</span></div><div className="deployment-grid"><div className="deployment-table"><table><thead><tr><th>{t('provider')}</th><th>{t('status')}</th><th>{t('addresses')}</th><th>{t('nodeCount')}</th><th>{t('lastSeen')}</th></tr></thead><tbody>{providers.length ? providers.map(provider => <tr key={provider.provider_id}><td><div className="token-cell"><span className="token-key"><ServerCog size={14} /></span><span><strong>{provider.name}</strong><small>{provider.provider_id} · v{provider.version}</small></span></div></td><td><span className={`status-badge ${provider.online ? 'good' : 'off'}`}><i /> {provider.online ? t('online') : provider.enabled ? t('stale') : t('disabled')}</span></td><td><div className="address-stack">{provider.addresses?.length ? provider.addresses.map(address => <span key={`${address.kind}-${address.value}`}><b>{address.kind}</b> {address.value}</span>) : <span className="muted-text">{t('noAdvertisedAddress')}</span>}</div></td><td>{provider.node_count}</td><td>{formatDate(provider.last_seen, language, t('never'))}</td></tr>) : <tr><td colSpan={5} className="empty">{t('noProvidersYet')}</td></tr>}</tbody></table></div><div className="deployment-nodes"><div className="panel-heading"><div><h2>{t('nodeInventory')}</h2><p>{t('nodeDetails')}</p></div><span className="select-button">{nodes.length}</span></div><div className="node-list compact-node-list">{nodes.length ? nodes.map(node => <div className="node-row" key={`${node.name}-${node.server}-${node.port}`}><span className="node-icon"><Server size={15} /></span><span className="node-main"><strong>{node.name}</strong><small>{node.server}:{node.port}</small></span><span className="node-latency">{node.protocol}<small>{t('online')}</small></span></div>) : <div className="empty">{t('noNodes')}</div>}</div></div></div></section></>
}

function TokenTable({ tokens, full, quickCopy, auth, onChanged, onToast, language, t }) { const hasActions = full || quickCopy; return <div className="table-wrap"><table><thead><tr><th>{t('token')}</th><th>{t('usage')}</th><th>{t('limit')}</th><th>{t('quotaPeriod')}</th><th>{t('expiresAt')}</th><th>{t('status')}</th>{hasActions && <th aria-label={t('status')} />}</tr></thead><tbody>{tokens.length ? tokens.map(record => <TokenRow key={record.token} record={record} full={full} showQuickCopy={quickCopy} auth={auth} onChanged={onChanged} onToast={onToast} language={language} t={t} />) : <tr><td colSpan={hasActions ? 7 : 6} className="empty">{t('noTokens')}</td></tr>}</tbody></table></div> }

function TokenRow({ record, full, showQuickCopy, auth, onChanged, onToast, language, t }) {
  const used = pct(record.used_bytes, record.limit_bytes)
  const [renewOpen, setRenewOpen] = useState(false)
  const [copying, setCopying] = useState(false)
  const [generated, setGenerated] = useState('')
  const disable = async () => { try { await request(`/api/admin/tokens/${encodeURIComponent(record.token)}`, { method: 'POST' }, auth); onChanged(); onToast(t('tokenDisabled')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }
  const reset = async () => { try { await request(`/api/admin/tokens/${encodeURIComponent(record.token)}/reset`, { method: 'POST' }, auth); onChanged(); onToast(t('reset')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }
  const renew = async duration => { try { await request(`/api/admin/tokens/${encodeURIComponent(record.token)}/renew`, { method: 'POST', body: JSON.stringify({ duration }) }, auth); setRenewOpen(false); onChanged(); onToast(t('renewal')) } catch (error) { onToast(t('operationFailed', { message: error.message })) } }
  const generateAndCopy = async () => {
    setCopying(true)
    try {
      const data = await request(`/api/admin/tokens/${encodeURIComponent(record.token)}/rotate`, { method: 'POST' }, auth)
      const url = new URL('/sub/c', PUBLIC_SUBSCRIPTION_ORIGIN)
      url.search = new URLSearchParams({ token: data.token || record.token, password: data.password, filename: record.label || 'vsub233' }).toString()
      setGenerated(url.toString())
      if (globalThis.navigator?.clipboard?.writeText) await globalThis.navigator.clipboard.writeText(url.toString())
      else {
        const input = globalThis.document?.createElement('textarea')
        if (!input) throw new Error('clipboard unavailable')
        input.value = url.toString(); input.style.position = 'fixed'; input.style.opacity = '0'; globalThis.document.body.appendChild(input); input.select()
        if (!globalThis.document.execCommand('copy')) throw new Error('clipboard unavailable')
        input.remove()
      }
      setGenerated(''); onChanged(); onToast(t('quickCopyDone'))
    } catch (error) { onToast(error.message.includes('clipboard') ? t('copyFailed') : t('operationFailed', { message: error.message })) }
    finally { setCopying(false) }
  }
  const expired = !record.enabled && record.disabled_reason === 'expired'
  return <><tr><td><div className="token-cell"><span className="token-key"><KeyRound size={14} /></span><span><strong>{record.token}</strong><small>{record.other_uid || t('passwordProtected')}</small></span></div></td><td><div className="usage-cell"><div className="usage-bar"><span style={{ width: `${used}%` }} /></div><small>{bytes(record.used_bytes)} / {bytes(record.limit_bytes)} · {formatDate(record.next_reset_at, language, t('never'))}</small></div></td><td>{bytes(record.limit_bytes)}</td><td><span className="period-cell">{periodLabel(record.quota_period, t)}</span></td><td><small>{record.expires_at ? formatDate(record.expires_at, language) : t('neverExpires')}</small></td><td><span className={`status-badge ${record.enabled ? 'good' : 'off'}`}><i /> {record.enabled ? t('active') : expired ? t('expired') : t('disabled')}</span></td>{(full || showQuickCopy) && <td><div className="table-actions">{showQuickCopy && <button className="table-action quick-copy-action" onClick={generateAndCopy} disabled={copying || !record.enabled} title={t('quickCopySecurityNote')}><Copy size={12} className={copying ? 'spin' : ''} /> {copying ? t('copyLink') : t('generateAndCopy')}</button>}{full && <><button className="table-action" onClick={() => setRenewOpen(!renewOpen)}>{t('renewal')}</button><button className="table-action" onClick={reset}>{t('reset')}</button><button className="table-action" onClick={disable} disabled={!record.enabled}>{record.enabled ? t('disable') : t('disabled')}</button>{renewOpen && <div className="renew-menu"><button onClick={() => renew('seven_days')}>{t('renew7')}</button><button onClick={() => renew('one_month')}>{t('renewMonth')}</button><button onClick={() => renew('one_year')}>{t('renewYear')}</button></div>}</>}</div></td>}</tr>{generated && <tr><td colSpan={7} className="quick-copy-recovery"><div className="security-note">{t('copyFailed')}</div><div className="generated-link-row"><input className="generated-link-input" readOnly value={generated} aria-label={t('subscriptionLink')} /><button type="button" className="secondary-button" onClick={async () => { try { await navigator.clipboard.writeText(generated); setGenerated(''); onToast(t('copied')) } catch { onToast(t('copyFailed')) } }}><Copy size={13} /> {t('copyLink')}</button></div></td></tr>}</>
}

function CreateToken({ auth, onClose, onCreated, onToast, t }) {
  const [form, setForm] = useState({ token: '', password: '', limit: '107374182400', expires: '', quotaPeriod: 'monthly_from_creation', duration: 'one_month', otherUid: '', profileName: 'vsub233' })
  const [subscriptionLink, setSubscriptionLink] = useState('')
  const [copied, setCopied] = useState(false)
  const [qrDataUrl, setQrDataUrl] = useState('')
  const [saving, setSaving] = useState(false)
  const update = (field, value) => setForm(current => ({ ...current, [field]: value }))
  const submit = async event => {
    event.preventDefault()
    setSaving(true)
    try {
      await request('/api/admin/tokens', { method: 'POST', body: JSON.stringify({ token: form.token, password: form.password, profileName: form.profileName.trim() || 'vsub233', limit_bytes: Number(form.limit), expires_at: form.expires ? Math.floor(new Date(form.expires).getTime() / 1000) : null, quota_period: form.quotaPeriod, duration: form.expires ? undefined : form.duration, other_uid: form.otherUid || null }) }, auth)
      const url = new URL('/sub/c', PUBLIC_SUBSCRIPTION_ORIGIN)
      url.search = new URLSearchParams({ token: form.token.trim(), password: form.password, filename: form.profileName.trim() || 'vsub233' }).toString()
      setSubscriptionLink(url.toString())
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
  const qrFilename = `${(form.profileName.trim() || 'vsub233').replace(/[^a-z0-9_-]+/gi, '_')}-shadowrocket.png`
  return <div className="modal-backdrop" onMouseDown={onClose}><div className="modal" onMouseDown={event => event.stopPropagation()}><div className="modal-head"><div><p className="eyebrow">{t('newPolicy')}</p><h2>{t('createToken')}</h2></div><button className="icon-button" title={t('close')} onClick={onClose}><X size={18} /></button></div><form onSubmit={submit} className="modal-form"><label>{t('token')}<input required disabled={Boolean(subscriptionLink)} value={form.token} placeholder={t('tokenPlaceholder')} onChange={e => update('token', e.target.value)} /></label><label>{t('subscriptionPassword')}<input required disabled={Boolean(subscriptionLink)} type="password" value={form.password} placeholder={t('passwordPlaceholder')} onChange={e => update('password', e.target.value)} /></label><label>{t('profileName')}<input required disabled={Boolean(subscriptionLink)} value={form.profileName} placeholder={t('profileNamePlaceholder')} onChange={e => update('profileName', e.target.value)} /></label><label>{t('quotaBytes')}<input required disabled={Boolean(subscriptionLink)} type="number" min="1" value={form.limit} onChange={e => update('limit', e.target.value)} /></label><label>{t('quotaPeriod')}<select disabled={Boolean(subscriptionLink)} value={form.quotaPeriod} onChange={e => update('quotaPeriod', e.target.value)}><option value="calendar_month">{t('calendarMonth')}</option><option value="monthly_from_creation">{t('creationMonth')}</option><option value="weekly">{t('weekly')}</option><option value="daily">{t('daily')}</option><option value="lifetime">{t('lifetime')}</option></select></label><label>{t('duration')}<select disabled={Boolean(subscriptionLink)} value={form.duration} onChange={e => update('duration', e.target.value)}><option value="seven_days">{t('sevenDays')}</option><option value="one_month">{t('oneMonth')}</option><option value="one_year">{t('oneYear')}</option></select></label><label>{t('expiresOptional')}<input disabled={Boolean(subscriptionLink)} type="datetime-local" value={form.expires} onChange={e => update('expires', e.target.value)} /></label><label>{t('otherUid')}<input disabled={Boolean(subscriptionLink)} value={form.otherUid} placeholder="order_xxx / user_xxx" onChange={e => update('otherUid', e.target.value)} /></label>{subscriptionLink && <><section className="generated-link-panel"><div><strong>{t('subscriptionLink')}</strong><p className="security-note">{t('linkSecurityNote')}</p></div><div className="generated-link-row"><input className="generated-link-input" readOnly value={subscriptionLink} aria-label={t('subscriptionLink')} /><button type="button" className="secondary-button" onClick={copyLink}><Copy size={15} /> {copied ? t('copied') : t('copyLink')}</button></div></section><section className="qr-panel"><div className="qr-copy"><strong>{t('qrCodeTitle')}</strong><p>{t('shadowrocketScan')}</p><small>{t('qrLocalNote')}</small></div><div className="qr-preview">{qrDataUrl ? <img src={qrDataUrl} alt={t('qrCodeTitle')} /> : <span><QrCode size={28} /> {t('qrGenerating')}</span>}</div>{qrDataUrl && <a className="secondary-button qr-download" href={qrDataUrl} download={qrFilename}><Download size={15} /> {t('downloadQr')}</a>}</section></>}<div className="modal-foot"><button type="button" className="secondary-button" onClick={onClose}>{t('close')}</button><button type="submit" className="primary-button" disabled={saving || Boolean(subscriptionLink)}><Check size={16} /> {saving ? t('create') : t('create')}</button></div></form></div></div>
}

function SectionPlaceholder({ title, nodes, tokens, onRefresh, t }) { return <><PageTitle title={title} copyText={t('moduleReady')} action={<button className="secondary-button" onClick={onRefresh}><RefreshCw size={16} /> {t('refresh')}</button>} /><section className="empty-section"><div className="empty-orbit"><Network size={25} /></div><h2>{t('moduleWorkspace', { title })}</h2><p>{t('serviceSummary', { nodes: nodes.length, tokens: tokens.length })}</p><button className="primary-button" onClick={onRefresh}>{t('refreshData')}</button></section></> }

const root = globalThis.__vpn233Root || (globalThis.__vpn233Root = createRoot(document.getElementById('root')))
root.render(<App />)
