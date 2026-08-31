use std::path::Path;

use serde_json::json;
use wae_engine::Analysis;

use crate::CliOutput;

pub(super) fn write(root: &Path, output: &Path, analysis: &Analysis) -> CliOutput {
    let path = if output.is_absolute() { output.to_path_buf() } else { root.join(output) };
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return CliOutput::project_error(format!(
                "cannot create explorer directory `{}`: {error}",
                parent.display()
            ));
        }
    }

    let modules = analysis
        .project
        .modules
        .iter()
        .map(|module| {
            let ownership = analysis.ownership.get(&module.id);
            json!({
                "id": module.id.0,
                "package": module.package.0,
                "kind": format!("{:?}", module.kind),
                "layer": module.layer.as_ref().map(|layer| &layer.0),
                "ownership": ownership,
                "runtime": format!("{:?}", module.runtime),
                "framework": module.framework_metadata.adapter_id,
                "frameworkAttributes": module.framework_metadata.attributes,
            })
        })
        .collect::<Vec<_>>();
    let edges = analysis
        .project
        .dependencies
        .iter()
        .map(|edge| {
            json!({
                "from": edge.from.0,
                "to": edge.to.0,
                "kind": format!("{:?}", edge.kind),
            })
        })
        .collect::<Vec<_>>();
    let model = json!({
        "schemaVersion": analysis.schema_version,
        "modules": modules,
        "edges": edges,
        "diagnostics": analysis.diagnostics,
    });
    // The model is data, not executable JavaScript. Escaping `<` also prevents a path containing
    // `</script>` from terminating the application/json element.
    let data = match serde_json::to_string(&model) {
        Ok(value) => value.replace('<', "\\u003c"),
        Err(error) => return CliOutput::internal_error(error.to_string()),
    };
    let html = TEMPLATE.replace("__WAE_MODEL__", &data);
    match std::fs::write(&path, html) {
        Ok(()) => CliOutput::success(format!(
            "Architecture Explorer: {} modules, {} dependencies, {} diagnostics\n{}",
            analysis.project.modules.len(),
            analysis.project.dependencies.len(),
            analysis.diagnostics.len(),
            path.display()
        )),
        Err(error) => {
            CliOutput::project_error(format!("cannot write explorer `{}`: {error}", path.display()))
        }
    }
}

const TEMPLATE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>WAE Architecture Explorer</title>
<style>
:root{color-scheme:dark;--bg:#07111f;--panel:#0d1b2e;--line:#29415e;--text:#e8f0fa;--muted:#8da3bd;--accent:#58d6b5;--danger:#ff667f}*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:14px ui-sans-serif,system-ui,sans-serif}header{height:64px;display:flex;align-items:center;gap:24px;padding:0 22px;border-bottom:1px solid var(--line);background:#091626}h1{font-size:17px;margin:0}header span{color:var(--muted)}main{display:grid;grid-template-columns:270px minmax(400px,1fr) 330px;height:calc(100vh - 64px)}aside{padding:18px;border-right:1px solid var(--line);overflow:auto}.details{border-left:1px solid var(--line);border-right:0}label{display:block;margin:14px 0 5px;color:var(--muted);font-size:12px}input,select{width:100%;background:#07111f;border:1px solid var(--line);border-radius:7px;color:var(--text);padding:9px}#canvas{position:relative;overflow:auto;background-image:radial-gradient(#19304a 1px,transparent 1px);background-size:24px 24px}svg{min-width:100%;min-height:100%}.node{cursor:pointer}.node rect{fill:#10243a;stroke:#3d5e80;stroke-width:1.5}.node:hover rect,.node.selected rect{stroke:var(--accent);stroke-width:3}.node.violation rect{stroke:var(--danger)}.node text{fill:var(--text);font-size:11px}.edge{stroke:#47627e;stroke-width:1;opacity:.65;fill:none}.pill{display:inline-block;padding:3px 7px;margin:2px;border-radius:99px;background:#18314c;color:#b8cae0;font-size:11px}.bad{color:var(--danger)}pre{white-space:pre-wrap;word-break:break-word;color:#b8cae0}button{background:var(--accent);border:0;border-radius:7px;padding:8px 10px;color:#062019;cursor:pointer}.empty{color:var(--muted);padding:20px}
</style></head><body><header><h1>WAE Architecture Explorer</h1><span id="summary"></span><button id="fit">Fit graph</button></header>
<main><aside><label for="search">Module search</label><input id="search" placeholder="src/features/auth…"><label for="package">Package</label><select id="package"><option value="">All packages</option></select><label for="layer">Layer</label><select id="layer"><option value="">All layers</option></select><label for="runtime">Runtime</label><select id="runtime"><option value="">All runtimes</option></select><label><input id="violations" type="checkbox" style="width:auto"> Only modules with violations</label><p id="visible"></p></aside><section id="canvas"><svg id="graph"></svg></section><aside class="details"><h2>Details</h2><div id="details" class="empty">Select a module.</div></aside></main>
<script id="wae-model" type="application/json">__WAE_MODEL__</script><script>
const model=JSON.parse(document.getElementById('wae-model').textContent),svg=document.getElementById('graph'),NS='http://www.w3.org/2000/svg';const state={search:'',package:'',layer:'',runtime:'',violations:false,selected:null};const diagnosticsByModule=new Map();for(const d of model.diagnostics){const paths=new Set([d.primary_location?.file,...(d.dependency_path||[])]);for(const p of paths){if(!p)continue;if(!diagnosticsByModule.has(p))diagnosticsByModule.set(p,[]);diagnosticsByModule.get(p).push(d)}}
function options(id,values){const el=document.getElementById(id);for(const v of [...new Set(values.filter(Boolean))].sort()){const o=document.createElement('option');o.value=o.textContent=v;el.append(o)}}options('package',model.modules.map(x=>x.package));options('layer',model.modules.map(x=>x.layer));options('runtime',model.modules.map(x=>x.runtime));for(const id of ['search','package','layer','runtime'])document.getElementById(id).addEventListener('input',e=>{state[id]=e.target.value.toLowerCase();render()});document.getElementById('violations').addEventListener('change',e=>{state.violations=e.target.checked;render()});
function render(){svg.replaceChildren();const nodes=model.modules.filter(n=>(!state.search||n.id.toLowerCase().includes(state.search))&&(!state.package||n.package.toLowerCase()===state.package)&&(!state.layer||(n.layer||'').toLowerCase()===state.layer)&&(!state.runtime||n.runtime.toLowerCase()===state.runtime)&&(!state.violations||diagnosticsByModule.has(n.id)));const ids=new Set(nodes.map(n=>n.id)),cols=Math.max(1,Math.ceil(Math.sqrt(nodes.length))),w=260,h=90,pos=new Map();nodes.forEach((n,i)=>pos.set(n.id,{x:35+(i%cols)*w,y:35+Math.floor(i/cols)*h}));svg.setAttribute('viewBox',`0 0 ${Math.max(700,cols*w+40)} ${Math.max(500,Math.ceil(nodes.length/cols)*h+60)}`);for(const e of model.edges){if(!ids.has(e.from)||!ids.has(e.to))continue;const a=pos.get(e.from),b=pos.get(e.to),line=document.createElementNS(NS,'path');line.setAttribute('class','edge');line.setAttribute('d',`M${a.x+190},${a.y+25} C${(a.x+b.x)/2+95},${a.y+25} ${(a.x+b.x)/2+95},${b.y+25} ${b.x},${b.y+25}`);svg.append(line)}for(const n of nodes){const p=pos.get(n.id),g=document.createElementNS(NS,'g');g.setAttribute('class',`node ${diagnosticsByModule.has(n.id)?'violation':''} ${state.selected===n.id?'selected':''}`);g.setAttribute('transform',`translate(${p.x} ${p.y})`);const rect=document.createElementNS(NS,'rect');rect.setAttribute('width','190');rect.setAttribute('height','54');rect.setAttribute('rx','8');const text=document.createElementNS(NS,'text');text.setAttribute('x','10');text.setAttribute('y','21');text.textContent=n.id.length>27?'…'+n.id.slice(-26):n.id;const meta=document.createElementNS(NS,'text');meta.setAttribute('x','10');meta.setAttribute('y','41');meta.setAttribute('fill','#8da3bd');meta.textContent=`${n.layer||'unowned'} · ${n.runtime}`;g.append(rect,text,meta);g.onclick=()=>select(n);svg.append(g)}document.getElementById('visible').textContent=`${nodes.length} visible modules`;document.getElementById('summary').textContent=`${model.modules.length} modules · ${model.edges.length} edges · ${model.diagnostics.length} diagnostics`}
function esc(v){return String(v??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]))}function select(n){state.selected=n.id;const ds=diagnosticsByModule.get(n.id)||[],incoming=model.edges.filter(e=>e.to===n.id),outgoing=model.edges.filter(e=>e.from===n.id);document.getElementById('details').innerHTML=`<h3>${esc(n.id)}</h3><span class="pill">${esc(n.package)}</span><span class="pill">${esc(n.layer||'unowned')}</span><span class="pill">${esc(n.runtime)}</span><span class="pill">${esc(n.framework||'framework-neutral')}</span><p>${incoming.length} incoming · ${outgoing.length} outgoing</p><h3 class="${ds.length?'bad':''}">Diagnostics (${ds.length})</h3>${ds.map(d=>`<p><b>${esc(d.rule_id)}</b> ${esc(d.message)}</p>`).join('')||'<p>None</p>'}<h3>Framework metadata</h3><pre>${esc(JSON.stringify(n.frameworkAttributes,null,2))}</pre>`;render()}document.getElementById('fit').onclick=()=>svg.scrollIntoView({block:'center',inline:'center'});render();
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use wae_core::domain::{
        Module, ModuleId, ModuleKind, ModulePath, PackageName, Project, Runtime,
    };

    #[test]
    fn writes_a_self_contained_escaped_document() {
        let root = std::env::temp_dir().join(format!("wae-explorer-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let project = Project {
            modules: vec![Module {
                id: ModuleId("src/</script>.ts".into()),
                path: ModulePath("src/</script>.ts".into()),
                package: PackageName("app".into()),
                kind: ModuleKind::Source,
                runtime: Runtime::Universal,
                layer: None,
                framework_metadata: Default::default(),
            }],
            ..Project::default()
        };
        let analysis = Analysis {
            schema_version: 1,
            graph: Default::default(),
            ownership: Default::default(),
            project,
            diagnostics: Vec::new(),
            incremental: Default::default(),
            timings: Default::default(),
        };
        let result = write(&root, Path::new("report/index.html"), &analysis);
        assert_eq!(result.exit_code, 0);
        let document = std::fs::read_to_string(root.join("report/index.html")).unwrap();
        assert!(document.contains("WAE Architecture Explorer"));
        assert!(document.contains("src/\\u003c/script>.ts"));
        assert!(!document.contains("src/</script>.ts"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
