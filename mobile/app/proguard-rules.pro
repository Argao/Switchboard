# R8/ProGuard rules for the release build.

# JNI: the Rust core resolves these by name (Java_com_aiyu_switchboard_core_NativeCore_*).
-keepclasseswithmembernames class com.aiyu.switchboard.core.NativeCore {
    native <methods>;
}

# kotlinx.serialization ships its own consumer rules; keep the sealed wire types' names stable.
-keep,includedescriptorclasses class com.aiyu.switchboard.net.**$$serializer { *; }
-keepclassmembers class com.aiyu.switchboard.** { *** Companion; }

# ML Kit lists its registrars in the manifest and builds them by reflection.
# R8 in full mode keeps the classes but drops a constructor nothing calls, so
# the barcode scanner was never registered and opening the camera crashed.
-keep class * implements com.google.firebase.components.ComponentRegistrar { <init>(); }
