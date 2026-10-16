// 模型只能填写源码；调用者明确提供契约，运行时重新验证并按原代数提交。
fn invoke(input, context) {
    let head = call_capability("/v1/runtime/enhancements/head", #{});
    if head.generation != input.expected_generation {
        throw "活动版本已变化，必须重新审阅";
    }
    let current = call_capability("/v1/runtime/enhancements/package", #{revision: head.revision});
    let target = input.target;
    let available = call_capability("/v1/runtime/enhancements/catalog", #{paths: target.capabilities});
    if available.len() != target.capabilities.len() {
        throw "请求依赖的能力未装配或未授权";
    }
    let generated = call_capability("/v1/model/enhancement/source", #{
        requirement: input.requirement, target: target, available: available
    });
    target.source = generated.source;
    let replaced = false;
    for index in 0..current.enhancements.len() {
        if current.enhancements[index].path == target.path {
            current.enhancements[index] = target;
            replaced = true;
        }
    }
    if !replaced {
        current.enhancements.push(target);
    }
    let proposed = call_capability("/v1/runtime/enhancements/propose", current);
    call_capability("/v1/runtime/enhancements/activate", #{
        revision: proposed.revision, expected_generation: head.generation
    })
}
