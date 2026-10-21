# Parse private runtime metadata as data, never as PowerShell expressions.
function Read-Vpn233Layout {
    param([string]$Base)
    $file = Join-Path $Base '.vpn233-layout'
    if (-not (Test-Path -LiteralPath $file)) { return $null }
    $item = Get-Item -LiteralPath $file
    if ($item.PSIsContainer -or $item.Attributes -band [IO.FileAttributes]::ReparsePoint -or $item.Length -gt 65536) { throw 'Invalid private runtime layout file.' }
    $lines = [IO.File]::ReadAllLines($file)
    if ($lines.Count -ne 6 -or $lines[0] -cne 'vpn233-layout-v1') { throw 'Invalid private runtime layout format.' }
    $keys = @('scope', 'bin', 'web', 'config', 'data')
    $layout = @{}
    for ($index = 0; $index -lt $keys.Count; $index++) {
        $fields = $lines[$index + 1].Split("`t")
        if ($fields.Count -ne 2 -or $fields[0] -cne $keys[$index] -or -not $fields[1] -or $fields[1] -match '[\x00-\x1f\x7f]') { throw 'Invalid private runtime layout entry.' }
        if ($index -eq 0) { if ($fields[1] -notin @('user', 'system', 'source')) { throw 'Invalid installation scope.' } }
        else {
            $absolute = if ($env:OS -eq 'Windows_NT') { $fields[1] -match '^(?:[A-Za-z]:[\\/]|\\\\[^\\/]+[\\/][^\\/]+)' } else { $fields[1].StartsWith('/') }
            if (-not $absolute) { throw 'Runtime layout must contain absolute paths.' }
        }
        $layout[$keys[$index]] = $fields[1]
    }
    return $layout
}
function Save-Vpn233Layout {
    param([string]$Base, [string]$Scope, [string]$Bin, [string]$Web, [string]$Config, [string]$Data)
    if ($Scope -notin @('user', 'system', 'source')) { throw 'Invalid installation scope.' }
    $values = @($Bin, $Web, $Config, $Data) | ForEach-Object {
        if ($_ -match '[\x00-\x1f\x7f]') { throw 'Runtime layout paths cannot contain control characters.' }
        [IO.Path]::GetFullPath($_)
    }
    $text = "vpn233-layout-v1`nscope`t$Scope`nbin`t$($values[0])`nweb`t$($values[1])`nconfig`t$($values[2])`ndata`t$($values[3])`n"
    $file = Join-Path $Base '.vpn233-layout'
    $temporary = Join-Path $Base ('.vpn233-layout.' + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::WriteAllText($temporary, $text, (New-Object Text.UTF8Encoding $false))
        if ($env:OS -eq 'Windows_NT') {
            $acl = Get-Acl -LiteralPath $temporary
            $acl.SetAccessRuleProtection($true, $false)
            foreach ($rule in @($acl.Access)) { [void]$acl.RemoveAccessRuleSpecific($rule) }
            $current = [Security.Principal.WindowsIdentity]::GetCurrent().User
            foreach ($sid in @($current, (New-Object Security.Principal.SecurityIdentifier 'S-1-5-18'), (New-Object Security.Principal.SecurityIdentifier 'S-1-5-32-544'))) {
                $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid, 'FullControl', 'Allow')))
            }
            Set-Acl -LiteralPath $temporary -AclObject $acl
        } else { & chmod 600 -- $temporary; if ($LASTEXITCODE -ne 0) { throw 'Unable to protect runtime layout.' } }
        Move-Item -LiteralPath $temporary -Destination $file -Force
    } finally { if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force } }
}
