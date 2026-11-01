$ErrorActionPreference = 'Stop'
python (Join-Path $PSScriptRoot 'install_engine.py')
if ($LASTEXITCODE -ne 0) { throw 'EasyTier 安装失败' }
