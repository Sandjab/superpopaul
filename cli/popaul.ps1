<#
.SYNOPSIS
    Popaul (PowerShell) - resout des adressages Peppol par fournees via l'API /resolve/batch.

.DESCRIPTION
    Version Windows/PowerShell de popaul.py. Lit une liste de Participant IDs
    (fichier texte, un par ligne, ou colonne d'un CSV), les envoie a l'API par
    paquets (<= 50), et ecrit un CSV : exists / code PA / nom PA / pays /
    support EXTENDED-CTC-FR. Gere l'auth par cle, les 429 (retry + backoff),
    la reprise (-Resume) et une barre de progression (Write-Progress).

    Compatible Windows PowerShell 5.1 et PowerShell 7+. Aucune dependance.

.EXAMPLE
    .\popaul.ps1 adressages.txt -Url https://peppol.gavini.cloud -Key MA_CLE -Output resultats.csv

.EXAMPLE
    .\popaul.ps1 entreprises.csv -Column pid -Url https://peppol.gavini.cloud -Key MA_CLE -Output out.csv -Resume

.NOTES
    La cle peut venir de l'environnement : $env:PEPPOL_API_KEY.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Path,                              # fichier de PID, ou '-' pour stdin
    [Parameter(Mandatory = $true)]
    [string]$Url,
    [string]$Key = $env:PEPPOL_API_KEY,
    [string]$Output,
    [string]$Column,                            # CSV : nom d'en-tete ou index 0-based
    [int]$BatchSize = 50,
    [switch]$Test,
    [switch]$Resume,
    [int]$TimeoutSec = 60,
    [int]$MaxRetries = 4
)

$ErrorActionPreference = 'Stop'
$BATCH_MAX = 50

function Get-Canonical([string]$p) {
    $p = $p.Trim()
    if ($p -like '*::*') { return $p } else { return "iso6523-actorid-upis::$p" }
}

function Format-Val($v) {
    if ($null -eq $v) { return '' }
    if ($v -is [bool]) { if ($v) { return 'true' } else { return 'false' } }
    return "$v"
}

function Coalesce($a, $b) {
    if ($null -ne $a -and "$a" -ne '') { return $a } else { return $b }
}

function Read-Participants([string]$Path, [string]$Column) {
    if ($Path -eq '-') {
        $lines = [Console]::In.ReadToEnd() -split "`r?`n"
    } else {
        $lines = Get-Content -LiteralPath $Path
    }
    if ($Column) {
        if ($Column -match '^\d+$') {
            # Index numerique : split naif sur la virgule (CSV simple, sans guillemets).
            $idx = [int]$Column
            return @($lines | Where-Object { $_.Trim() -ne '' } |
                     ForEach-Object { ($_ -split ',')[$idx] } |
                     Where-Object { $_ -and $_.Trim() -ne '' } |
                     ForEach-Object { $_.Trim() })
        } else {
            # Nom de colonne : parsing CSV propre (1re ligne = en-tete).
            return @($lines | ConvertFrom-Csv | ForEach-Object { $_.$Column } |
                     Where-Object { $_ -and $_.Trim() -ne '' } |
                     ForEach-Object { $_.Trim() })
        }
    }
    return @($lines | Where-Object { $_.Trim() -ne '' -and -not $_.TrimStart().StartsWith('#') } |
             ForEach-Object { $_.Trim() })
}

function Get-RetryDelay($err, [int]$attempt) {
    # Retry-After si present, sinon backoff exponentiel (plafonne a 30 s).
    try {
        $ra = $err.Exception.Response.Headers['Retry-After']
        if ($ra) { return [double]$ra }
    } catch { }
    return [math]::Min(30, [math]::Pow(2, $attempt))
}

function Invoke-Batch([string]$Base, [string]$Key, [string[]]$Chunk, [bool]$Test,
                      [int]$TimeoutSec, [int]$MaxRetries) {
    # Corps JSON : on serialise chaque PID individuellement pour garder un tableau
    # meme a un seul element (quirk de ConvertTo-Json sur les tableaux d'1 element).
    $partJson = ($Chunk | ForEach-Object { ConvertTo-Json $_ }) -join ','
    $body = '{"participants":[' + $partJson + '],"test":' + ($Test.ToString().ToLower()) + '}'
    $uri = ($Base.TrimEnd('/')) + '/resolve/batch'
    for ($attempt = 0; $attempt -le $MaxRetries; $attempt++) {
        try {
            $resp = Invoke-RestMethod -Method Post -Uri $uri -TimeoutSec $TimeoutSec `
                        -ContentType 'application/json' -Body $body `
                        -Headers @{ 'X-API-Key' = $Key }
            return @($resp.results)
        } catch {
            $code = 0
            try { $code = [int]$_.Exception.Response.StatusCode } catch { }
            if ($code -eq 401) { throw "ERREUR 401 : cle d'API manquante ou invalide." }
            if ($attempt -lt $MaxRetries -and ($code -eq 429 -or ($code -ge 500 -and $code -lt 600) -or $code -eq 0)) {
                $d = Get-RetryDelay $_ $attempt
                if ($code -eq 429) { Write-Host "  429 (rate limit) - nouvelle tentative dans $([math]::Round($d,1))s" -ForegroundColor DarkYellow }
                Start-Sleep -Seconds $d
                continue
            }
            $msg = if ($code) { "HTTP $code" } else { $_.Exception.Message }
            return @($Chunk | ForEach-Object { [pscustomobject]@{ participant = $_; error = $msg } })
        }
    }
    return @($Chunk | ForEach-Object { [pscustomobject]@{ participant = $_; error = 'echec apres retries' } })
}

function ConvertTo-Row($item, [string]$sent) {
    if ($item.PSObject.Properties.Name -contains 'error') {
        return [pscustomobject][ordered]@{
            participant = (Coalesce $item.participant $sent)
            exists = ''; pa_code = ''; pa_name = ''; pa_country = ''
            supports_extended_ctc_fr = ''; note = $item.error
        }
    }
    $pa = $item.pa
    return [pscustomobject][ordered]@{
        participant = (Coalesce $item.participant_id $sent)
        exists = (Format-Val $item.exists)
        pa_code = (Format-Val $pa.code)
        pa_name = (Format-Val $pa.name)
        pa_country = (Format-Val $pa.country)
        supports_extended_ctc_fr = (Format-Val $item.supports_extended_ctc_fr)
        note = (Format-Val $item.note)
    }
}

# --- validation ------------------------------------------------------------
if (-not $Key) { throw "ERREUR : cle d'API requise (-Key ou `$env:PEPPOL_API_KEY)." }
$size = [math]::Max(1, [math]::Min($BatchSize, $BATCH_MAX))
if ($BatchSize -gt $BATCH_MAX) { Write-Host "[popaul] batch-size ramene a $BATCH_MAX (limite serveur)." -ForegroundColor DarkYellow }

$pids = @(Read-Participants -Path $Path -Column $Column)
if ($pids.Count -eq 0) { throw "Aucun adressage en entree." }

# --- reprise ---------------------------------------------------------------
$append = $false
if ($Resume) {
    if (-not $Output) { throw "-Resume necessite -Output (le CSV a completer)." }
    if (Test-Path -LiteralPath $Output) {
        $done = @{}
        Import-Csv -LiteralPath $Output | Where-Object { $_.exists -ne '' } |
            ForEach-Object { $done[$_.participant] = $true }
        $before = $pids.Count
        $pids = @($pids | Where-Object { -not $done.ContainsKey((Get-Canonical $_)) })
        $append = $true
        Write-Host "[reprise] $($done.Count) deja resolus, $($before - $pids.Count) ignores, $($pids.Count) a traiter." -ForegroundColor Yellow
    }
}
if ($pids.Count -eq 0) { Write-Host "OK Rien a faire : tous les adressages sont deja resolus." -ForegroundColor Green; exit 0 }

# Ecrase la sortie si run neuf (sans reprise) : les fournees s'y ajoutent ensuite.
if ($Output -and -not $append -and (Test-Path -LiteralPath $Output)) { Remove-Item -LiteralPath $Output }

$total = $pids.Count
$nBatches = [int][math]::Ceiling($total / $size)
Write-Host "Popaul : $total adressages en $nBatches fournee(s) de $size -> $Url" -ForegroundColor Cyan

$counts = @{ exists = 0; absent = 0; ext = 0; error = 0 }
$allRows = New-Object System.Collections.Generic.List[object]
$processed = 0
$bn = 0
for ($i = 0; $i -lt $total; $i += $size) {
    $end = [math]::Min($i + $size, $total) - 1
    $chunk = @($pids[$i..$end])
    $bn++
    $results = @(Invoke-Batch -Base $Url -Key $Key -Chunk $chunk -Test $Test.IsPresent -TimeoutSec $TimeoutSec -MaxRetries $MaxRetries)
    $rows = for ($j = 0; $j -lt $chunk.Count; $j++) {
        $item = if ($j -lt $results.Count) { $results[$j] } else { [pscustomobject]@{ participant = $chunk[$j]; error = 'reponse tronquee' } }
        ConvertTo-Row $item $chunk[$j]
    }
    foreach ($r in $rows) {
        if ($r.note -and $r.exists -eq '') { $counts.error++ }
        elseif ($r.exists -eq 'true') { $counts.exists++; if ($r.supports_extended_ctc_fr -eq 'true') { $counts.ext++ } }
        elseif ($r.exists -eq 'false') { $counts.absent++ }
    }
    if ($Output) {
        $rows | Export-Csv -LiteralPath $Output -Append -NoTypeInformation -Encoding UTF8
    } else {
        $rows | ForEach-Object { $allRows.Add($_) }
    }
    $processed += $chunk.Count
    Write-Progress -Activity "Popaul" -Status "$processed/$total (fournee $bn/$nBatches)" `
        -PercentComplete ([int](100 * $processed / $total))
}
Write-Progress -Activity "Popaul" -Completed

if (-not $Output) { $allRows | ConvertTo-Csv -NoTypeInformation }

Write-Host ("OK Termine : {0} enregistres ({1} EXTENDED-CTC-FR), {2} absents, {3} en erreur." -f `
    $counts.exists, $counts.ext, $counts.absent, $counts.error) -ForegroundColor Green
if ($Output) { Write-Host "   Resultats -> $Output" -ForegroundColor Green }
