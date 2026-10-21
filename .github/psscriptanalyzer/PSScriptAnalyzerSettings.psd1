# PSScriptAnalyzer gate used by .github/workflows/ci.yml (scripts-windows job).
#
# The baseline severity is Warning so real problems fail CI. A small number of
# stylistic rules are excluded because this repository intentionally targets
# PowerShell 5.1 compatible operator scripts that report progress with
# Write-Host and share helper functions across install/update/deploy scripts.
@{
    Severity     = @('Error', 'Warning')
    ExcludeRules = @(
        # Operator scripts communicate with the person running them; Write-Host
        # is the PowerShell 5.1 compatible way to do that on the console.
        'PSAvoidUsingWriteHost',
        # Helper functions such as Get-Vpn233Arch or Install-Vpn233File are
        # domain helpers, not exported cmdlets; the approved-verb list does not
        # cover every reasonable name.
        'PSUseApprovedVerbs',
        # The scripts are linear installers, not cmdlets with -WhatIf/-Confirm.
        'PSUseShouldProcessForStateChangingFunctions',
        # Same reason as PSUseApprovedVerbs: helpers such as Resolve-* and
        # Convert-* do not need singular nouns.
        'PSUseSingularNouns'
    )
}
