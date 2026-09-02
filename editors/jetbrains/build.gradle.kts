import org.jetbrains.intellij.platform.gradle.IntelliJPlatformType

plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm") version "2.0.21"
    id("org.jetbrains.intellij.platform") version "2.2.1"
}

group = "com.donerfan.wae"
version = "0.0.25"

repositories {
    mavenCentral()
    intellijPlatform { defaultRepositories() }
}

dependencies {
    intellijPlatform {
        intellijIdeaUltimate("2024.2")
        pluginVerifier()
    }
}

kotlin { jvmToolchain(21) }

intellijPlatform {
    buildSearchableOptions = false
    pluginConfiguration { ideaVersion { sinceBuild = "242" } }
    pluginVerification {
        ides { ide(IntelliJPlatformType.IntellijIdeaUltimate, "2024.2") }
    }
}
