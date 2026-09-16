const { readFileSync } = require('node:fs');
const { parseEnv } = require('node:util');
const { Client } = require('/opt/aio-delivery/ops/node_modules/pg');

(async()=>{
  const settings=parseEnv(readFileSync('/opt/aio-idea/runtime.env','utf8'));
  const client=new Client({connectionString:settings.AIO_DATABASE_URL});
  await client.connect();
  try {
    if (process.argv[2]) {
      const id=Number(process.argv[2]);
      if (!Number.isSafeInteger(id)||id<1) throw new Error('任务 ID 必须为正整数');
      const result=await client.query('SELECT id,git,source_revision,state,package_revision,error,created_at,updated_at FROM delivery_jobs WHERE id=$1',[id]);
      console.log(JSON.stringify(result.rows,null,2));
      return;
    }
    const sources=await client.query('SELECT git,branch,desired_sha,enabled,updated_at FROM delivery_sources ORDER BY updated_at DESC LIMIT 20');
    const jobs=await client.query('SELECT id,git,source_revision,state,package_revision,left(error,1600) AS error,created_at,updated_at FROM delivery_jobs ORDER BY id DESC LIMIT 30');
    const rollouts=await client.query('SELECT tenant_id,source_id,revision,state,left(error,600) AS error,updated_at FROM delivery_rollouts ORDER BY updated_at DESC LIMIT 30');
    const versions=await client.query("SELECT p.git,p.revision,p.version,p.source_revision,p.created_at,pj.updated_at AS published_at,length(d.readme) AS readme_length FROM plugin_packages p JOIN delivery_sources s ON s.git=p.git LEFT JOIN plugin_documents d ON d.revision=p.revision LEFT JOIN plugin_publish_jobs pj ON pj.revision=p.revision AND pj.state='active' ORDER BY p.created_at DESC LIMIT 30");
    const activations=await client.query("SELECT e.tenant_id,s.git,r.revision,e.lifecycle,e.created_at FROM plugin_lifecycle_events e JOIN plugin_sources s ON s.id=e.source_id JOIN delivery_sources d ON d.git=s.git LEFT JOIN plugin_revisions r ON r.id=e.revision_id WHERE e.lifecycle IN ('activate','rollback','deactivate','uninstall') ORDER BY e.created_at DESC LIMIT 40");
    const components=await client.query("SELECT s.git,v.digest,v.source_commit,i.tenant_id,i.digest AS installed_digest,i.enabled,r.state AS rollout_state,left(r.error,600) AS error FROM component_sources s JOIN component_publications p ON p.source_id=s.id JOIN component_versions v ON v.digest=p.digest LEFT JOIN component_installations i ON i.source_id=s.id LEFT JOIN component_rollouts r ON r.source_id=i.source_id AND r.tenant_id=i.tenant_id AND r.digest=p.digest WHERE EXISTS(SELECT 1 FROM delivery_sources d WHERE d.git=s.git)");
    console.log(JSON.stringify({components:components.rows,sources:sources.rows,jobs:jobs.rows,rollouts:rollouts.rows,versions:versions.rows,activations:activations.rows},null,2));
  }finally{await client.end();}
})().catch(error=>{console.error(error.message);process.exitCode=1;});
