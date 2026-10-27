# Non-installing regression: read properties from a disposable MSI database.
param([Parameter(Mandatory = $true)][string]$Artifacts)
$ErrorActionPreference = 'Stop'
$source = Join-Path $PSScriptRoot 'windows_installer_acceptance.ps1'
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0) { throw 'Acceptance script failed to parse.' }
$functions = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Read-MsiProperty'
}, $true))
if ($functions.Count -ne 1) { throw 'Expected exactly one property reader.' }
# Load only this function; none of the hosted installer entry point is executed.
. ([scriptblock]::Create($functions[0].Extent.Text))
$root = Join-Path ([IO.Path]::GetFullPath($Artifacts)) ([Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$file = Join-Path $root 'property-fixture.msi'
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $null
$view = $null
try {
    # OpenDatabase mode 3 creates a file only. No msiexec or product registration.
    $database = $installer.OpenDatabase($file, 3)
    $view = $database.OpenView('CREATE TABLE `Property` (`Property` CHAR(72) NOT NULL, `Value` CHAR(0) LOCALIZABLE PRIMARY KEY `Property`)')
    [void]$view.Execute()
    [void]$view.Close()
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($view)
    $view = $null
    $expected = [ordered]@{
        ProductVersion = '0.4.0'
        ProductCode = '{FD1E4131-82EF-4D46-AC23-251AC19D832C}'
        UpgradeCode = '{C8BB512D-3155-4371-BAE9-83D676DF9B7B}'
    }
    foreach ($entry in $expected.GetEnumerator()) {
        $sql = "INSERT INTO ``Property`` (``Property``, ``Value``) VALUES ('$($entry.Key)', '$($entry.Value)')"
        $view = $database.OpenView($sql)
        [void]$view.Execute()
        [void]$view.Close()
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($view)
        $view = $null
    }
    [void]$database.Commit()
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($database)
    $database = $null
    foreach ($entry in $expected.GetEnumerator()) {
        $values = @(Read-MsiProperty $file $entry.Key)
        if ($values.Count -ne 1 -or $values[0] -isnot [string] -or $values[0] -cne $entry.Value) {
            throw "Property $($entry.Key) must emit exactly one matching scalar string; got $($values.Count) values."
        }
        if ($entry.Key -eq 'ProductVersion') {
            if ([version]$values[0] -ne [version]'0.4.0') { throw 'Version conversion failed.' }
        } else {
            [void][guid]$values[0]
        }
    }
    Write-Output 'PASS: ProductVersion, ProductCode and UpgradeCode emit one scalar string; no MSI installation performed.'
} finally {
    foreach ($com in @($view, $database, $installer)) {
        if ($null -ne $com) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($com) }
    }
    if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file -Force }
}
