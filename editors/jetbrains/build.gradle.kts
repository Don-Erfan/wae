plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm") version "2.0.21"
    id("org.jetbrains.intellij.platform") version "2.2.1"
}

group = "com.donerfan.wae"
version = "0.0.20"

repositories {
    mavenCentral()
    intellijPlatform { defaultRepositories() }
}

dependencies {
    intellijPlatform {
        webstorm("2024.2")
        bundledModule("com.intellij.modules.lsp")
        pluginVerifier()
    }
}

kotlin { jvmToolchain(21) }

intellijPlatform { pluginConfiguration { ideaVersion { sinceBuild = "242" } } }
