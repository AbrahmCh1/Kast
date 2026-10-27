import java.io.File
import java.util.Properties
import javax.inject.Inject
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Versión: -PkastVersion=1.6.0 (la CI pasa la etiqueta o dev-<sha>).
val kastVersion = (findProperty("kastVersion") as String?) ?: "dev"
val kastCode = (findProperty("kastVersionCode") as String?)?.toInt() ?: 1

android {
    namespace = "io.github.abrahmch1.kast"
    compileSdk = 36

    defaultConfig {
        applicationId = "io.github.abrahmch1.kast"
        minSdk = 26
        targetSdk = 34
        versionCode = kastCode
        versionName = kastVersion
    }

    // Firma de publicación si se dan los datos del almacén de claves (secretos de la CI); si no, la de depuración
    // (se instala igual, pero para actualizar hay que firmar siempre con la misma clave).
    val ks = System.getenv("KAST_ANDROID_KEYSTORE")
    signingConfigs {
        if (ks != null && File(ks).exists()) {
            create("release") {
                storeFile = File(ks)
                storePassword = System.getenv("KAST_ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("KAST_ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("KAST_ANDROID_KEY_PASSWORD") ?: System.getenv("KAST_ANDROID_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("rustJniLibs"))
}

kotlin {
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
}

// ---------------------------------------------------------------- núcleo en Rust (link/)

val linkDir = rootProject.projectDir.resolve("../link").canonicalFile
val abis = (findProperty("kastAbis") as String).split(",").map { it.trim() }.filter { it.isNotEmpty() }
val targets = mapOf(
    "arm64-v8a" to ("aarch64-linux-android" to "aarch64-linux-android"),
    "armeabi-v7a" to ("armv7-linux-androideabi" to "armv7a-linux-androideabi"),
    "x86_64" to ("x86_64-linux-android" to "x86_64-linux-android"),
)

/** NDK: ANDROID_NDK_HOME / ANDROID_NDK_ROOT o el más nuevo del SDK. */
fun ndkDir(): File {
    listOf("ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "ANDROID_NDK_LATEST_HOME").mapNotNull { System.getenv(it) }.map(::File)
        .firstOrNull { it.isDirectory }?.let { return it }
    val props = Properties()
    rootProject.file("local.properties").takeIf { it.exists() }?.inputStream()?.use(props::load)
    val sdk = props.getProperty("sdk.dir") ?: System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT")
        ?: error("no se encuentra el SDK de Android (local.properties: sdk.dir o ANDROID_HOME)")
    return File(sdk, "ndk").listFiles()?.filter { it.isDirectory }?.maxByOrNull { it.name }
        ?: error("no hay ningún NDK instalado en $sdk/ndk")
}

interface Injected { @get:Inject val exec: ExecOperations }
val injected = objects.newInstance<Injected>()

val cargoBuild by tasks.registering {
    description = "Compila link/ (Rust) como biblioteca JNI para cada ABI"
    inputs.dir(linkDir.resolve("src"))
    inputs.files(linkDir.resolve("Cargo.toml"), linkDir.resolve("Cargo.lock"))
    val out = layout.buildDirectory.dir("rustJniLibs")
    outputs.dir(out)
    doLast {
        val os = System.getProperty("os.name").lowercase()
        val host = when {
            os.contains("windows") -> "windows-x86_64"
            os.contains("mac") -> "darwin-x86_64"
            else -> "linux-x86_64"
        }
        val bin = ndkDir().resolve("toolchains/llvm/prebuilt/$host/bin")
        val ext = if (os.contains("windows")) ".cmd" else ""
        for (abi in abis) {
            val (triple, clang) = targets[abi] ?: error("ABI desconocida: $abi")
            val env = "CARGO_TARGET_${triple.uppercase().replace('-', '_')}_LINKER"
            injected.exec.exec {
                workingDir = linkDir
                environment(env, bin.resolve("$clang${android.defaultConfig.minSdk}-clang$ext").absolutePath)
                commandLine("cargo", "rustc", "--release", "--locked", "--lib", "--crate-type", "cdylib", "--target", triple)
            }
            val dest = out.get().dir(abi).asFile
            dest.mkdirs()
            linkDir.resolve("target/$triple/release/libkast_link.so").copyTo(dest.resolve("libkast_link.so"), overwrite = true)
        }
    }
}

tasks.named("preBuild") { dependsOn(cargoBuild) }
