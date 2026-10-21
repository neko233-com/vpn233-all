# Deterministic HTTP echo server used by scripts/e2e/e2e.ps1.
#
# It answers every GET request with a JSON document so the Windows E2E suite can
# prove that traffic really travelled through the vpn233-client SOCKS5/HTTP
# proxies. Implemented with a raw TcpListener so it works without the URL ACL
# that HttpListener needs on Windows.
#
# Usage:
#   powershell -NoProfile -File scripts/e2e/echo_server.ps1 -Port 18080
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][int]$Port,
    [string]$Bind = '127.0.0.1',
    [string]$Marker = 'vpn233-e2e-echo'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$listener = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Parse($Bind), $Port)
$listener.Start()
Write-Host "echo server listening on http://${Bind}:${Port}"

try {
    while ($true) {
        $client = $listener.AcceptTcpClient()
        try {
            $stream = $client.GetStream()
            $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::ASCII)
            $requestLine = $reader.ReadLine()
            if ([string]::IsNullOrEmpty($requestLine)) { continue }
            $parts = $requestLine.Split(' ')
            $path = '/'
            if ($parts.Length -ge 2) { $path = $parts[1] }
            # Consume the remaining request headers.
            while ($true) {
                $line = $reader.ReadLine()
                if ([string]::IsNullOrEmpty($line)) { break }
            }
            $payload = '{{"marker":"{0}","method":"GET","path":"{1}"}}' -f $Marker, $path
            $bytes = [System.Text.Encoding]::UTF8.GetBytes($payload + "`n")
            $header = "HTTP/1.1 200 OK`r`nContent-Type: application/json; charset=utf-8`r`nContent-Length: $($bytes.Length)`r`nCache-Control: no-store`r`nConnection: close`r`n`r`n"
            $headerBytes = [System.Text.Encoding]::ASCII.GetBytes($header)
            $stream.Write($headerBytes, 0, $headerBytes.Length)
            $stream.Write($bytes, 0, $bytes.Length)
            $stream.Flush()
        }
        finally {
            $client.Close()
        }
    }
}
finally {
    $listener.Stop()
}
