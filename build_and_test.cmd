@echo off
echo Running tests in submodules...
for /d %%D in (*) do (
  if exist "%%D\go.mod" (
    echo Testing %%D...
    pushd %%D
    go test ./... || (
      echo Tests failed in %%D
      popd
      exit /b 1
    )
    popd
  )
)
echo All tests passed.
echo Running packaging scripts...
call client-打包-windows-macos-linux.cmd
call server-打包-linux.cmd
echo Build and packaging finished.
pause
