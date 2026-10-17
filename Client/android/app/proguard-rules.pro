# ProGuard/R8 规则：release 关闭混淆，此文件保留扩展点。
# JNI 导出符号 Java_com_whatthemanddoing_app_NativeBridge_* 依赖类名稳定：
-keep class com.whatthemanddoing.app.NativeBridge { *; }
-keep class com.whatthemanddoing.app.ForegroundWatchService { *; }
-keep class com.whatthemanddoing.app.ForegroundService { *; }
-keep class com.whatthemanddoing.app.BootReceiver { *; }
