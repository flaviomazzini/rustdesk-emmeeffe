# EMME EFFE - modifiche al submodulo libs/hbb_common (codice di RustDesk, non nostro: si patcha al momento della build).
#  - solo in ingresso: is_incoming_only() sempre true
#  - server e chiave Emme Effe come predefiniti (come nelle 1.5.x compilate)
$ErrorActionPreference = "Stop"
$path = "libs/hbb_common/src/config.rs"
$c = [IO.File]::ReadAllText($path)

$n = [regex]::Replace($c, 'pub fn is_incoming_only\(\) -> bool \{.*?\n\}', "pub fn is_incoming_only() -> bool {`n    true`n}", [Text.RegularExpressions.RegexOptions]::Singleline)
if ($n -eq $c) { throw "is_incoming_only() non trovata" }
$c = $n

$sostituzioni = @{
  'pub const RENDEZVOUS_SERVERS: &[&str] = &["rs-ny.rustdesk.com"];' = 'pub const RENDEZVOUS_SERVERS: &[&str] = &["pbxemmeeffe.dyndns.org"];'
  'pub const RS_PUB_KEY: &str = "OeVuKk5nlHiXp+APNn0Y3pC1Iwpwn44JGqrQCsWqmBw=";' = 'pub const RS_PUB_KEY: &str = "4WTIAYQnc4BddzGns6tfwJimRXsKlsKmOjjWHegNtns=";'
}
foreach ($k in $sostituzioni.Keys) {
  if (!$c.Contains($k)) { throw "Non trovato in config.rs: $k" }
  $c = $c.Replace($k, $sostituzioni[$k])
}
[IO.File]::WriteAllText($path, $c)
Select-String -Path $path -Pattern 'RENDEZVOUS_SERVERS: |RS_PUB_KEY: ' | ForEach-Object { $_.Line }
Select-String -Path $path -Pattern 'pub fn is_incoming_only' -Context 0,2 | ForEach-Object { $_.Line; $_.Context.PostContext }
