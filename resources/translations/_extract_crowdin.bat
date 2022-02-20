@echo off
7z x -o"translations/" "gyroflow (translations).zip"

echo F | xcopy /y translations\da\gyroflow.ts da.ts
echo F | xcopy /y translations\de\gyroflow.ts de.ts
echo F | xcopy /y translations\el\gyroflow.ts el.ts
echo F | xcopy /y translations\es-ES\gyroflow.ts es.ts
echo F | xcopy /y translations\fi\gyroflow.ts fi.ts
echo F | xcopy /y translations\fr\gyroflow.ts fr.ts
echo F | xcopy /y translations\id\gyroflow.ts id.ts
echo F | xcopy /y translations\it\gyroflow.ts it.ts
echo F | xcopy /y translations\ja\gyroflow.ts ja.ts
echo F | xcopy /y translations\no\gyroflow.ts no.ts
echo F | xcopy /y translations\pl\gyroflow.ts pl.ts
echo F | xcopy /y translations\gl\gyroflow.ts gl.ts
echo F | xcopy /y translations\pt-PT\gyroflow.ts pt.ts
echo F | xcopy /y translations\pt-BR\gyroflow.ts pt_BR.ts
echo F | xcopy /y translations\sk\gyroflow.ts sk.ts
echo F | xcopy /y translations\uk\gyroflow.ts uk.ts
echo F | xcopy /y translations\ru\gyroflow.ts ru.ts
echo F | xcopy /y translations\zh-CN\gyroflow.ts zh_CN.ts
echo F | xcopy /y translations\zh-TW\gyroflow.ts zh_TW.ts
rmdir /s /q translations

call _release_texts.bat
del gyroflow.qm