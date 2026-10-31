#
# 把 PerchCore 的 Rust 核心静态链接进 Flutter App。
#
# 关键点（参考 cargokit 的脚本结构，链接参数位置以 CI 实测为准，见 docs/build.md「iOS 静态链接」）：
#
# 1. `perch_call` 没有任何 Swift/ObjC 代码引用它，不加 -force_load 会被链接器
#    当成死代码丢掉，于是运行时 DynamicLibrary.process() 找不到符号。
#    但 `-force_load` 只能加在 **user_target_xcconfig**（作用于 App 的链接命令行）：
#    pod target 的链接参数不会传到 App，不能只写在 pod_target_xcconfig 里。
#    ⚠️ 这一条与 cargokit 公开示例的写法不同，是 CI 实测后的修正（见 docs/build.md「iOS 静态链接」）。
#
# 2. script_phase 必须 execution_position = :before_compile，
#    并声明 input_files / output_files，否则 Xcode 不知道依赖关系，
#    可能"先链接后编译"。
#
# 3. 产物放在 **pod target 的** ${BUILT_PRODUCTS_DIR}/libperch_ffi.a，
#    即 .../<config>/perch_core_ffi/libperch_ffi.a；
#    -force_load 那边要写成 ${BUILT_PRODUCTS_DIR}/perch_core_ffi/libperch_ffi.a
#    （两个 BUILT_PRODUCTS_DIR 含义不同，见文件末尾的注释）。
#
Pod::Spec.new do |spec|
  spec.name             = 'perch_core_ffi'
  spec.version          = '0.1.0'
  spec.summary          = 'PerchCore 的 Rust 核心（静态链接）'
  spec.description      = <<-DESC
把 crates/perch-ffi 编译成 iOS 静态库并链接进 App。
插件本身不提供 MethodChannel —— 它的唯一职责是让 Rust 的 C ABI 符号
出现在最终二进制里，供 dart:ffi 的 DynamicLibrary.process() 解析。
                         DESC
  spec.homepage         = 'https://github.com/Sycamore-awa/PerchCore'
  spec.license          = { :type => 'MIT', :file => '../LICENSE' }
  spec.author           = { 'PerchCore' => 'noreply@example.com' }
  spec.source           = { :path => '.' }

  spec.source_files     = 'Classes/**/*'
  spec.dependency 'Flutter'
  spec.platform         = :ios, '18.0'

  spec.script_phase = {
    :name => 'Build Rust core',
    :script => '"${PODS_TARGET_SRCROOT}/build_rust.sh"',
    :execution_position => :before_compile,
    :input_files  => ['${PODS_TARGET_SRCROOT}/build_rust.sh'],
    :output_files => ['${BUILT_PRODUCTS_DIR}/libperch_ffi.a'],
  }

  # `DEFINES_MODULE` / `EXCLUDED_ARCHS` 作用在 **pod target** 上
  spec.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
  }

  # ⚠️ `-force_load` 必须放在 **user_target_xcconfig**（才会写进
  # Pods-Runner.xcconfig，作用于 App 的链接命令行）。写在 pod_target_xcconfig 里
  # 时它只影响 pod target，CI 实测 App 的 xcconfig 里根本没有这条参数。
  #
  # ⚠️ 路径必须带 `perch_core_ffi/` 这一层：
  # 脚本阶段在 **pod target** 里跑，它的 BUILT_PRODUCTS_DIR 是
  #   .../Debug-iphonesimulator/perch_core_ffi/
  # 而这里的 ${BUILT_PRODUCTS_DIR} 是 **App target** 的
  #   .../Debug-iphonesimulator/
  # 少写一层就会得到 "Build input file cannot be found"（CI 实测，见 docs/build.md「iOS 静态链接」）。
  spec.user_target_xcconfig = {
    'OTHER_LDFLAGS' => '-force_load ${BUILT_PRODUCTS_DIR}/perch_core_ffi/libperch_ffi.a',
  }
end
