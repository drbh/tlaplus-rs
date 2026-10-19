// Real browser coverage against `npm run dev` (or preview) on port 7821.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {chromium,expect} from '@playwright/test';
const browserMode=process.env.PLAYGROUND_BROWSER==='1';
const transport=browserMode?'**/checker.wasm':'**/api/check';
const url=process.env.PLAYGROUND_URL??'http://127.0.0.1:7821';
// CodeMirror virtualizes off-screen lines; select through its keyboard handler.
async function replaceSource(editor,source){await editor.click();await editor.press('ControlOrMeta+a');await editor.page().keyboard.insertText(source)}

test('edit both languages, inspect traces/graphs, preserve drafts and handle limits',async()=>{
  const browser=await chromium.launch({headless:true});
  try{
    const page=await browser.newPage({viewport:{width:1440,height:1000}});
    const errors=[];page.on('pageerror',e=>errors.push(e.message));
    await page.goto(url);
    const run=()=>page.getByRole('button',{name:'Run model →',exact:true}).click();
    await run();
    await expect(page.getByText('Invariant NotSolved is violated.',{exact:true})).toBeVisible();
    await expect(page.getByText('6 transitions',{exact:true})).toBeVisible();
    await page.getByRole('button',{name:'Next →',exact:true}).click();
    await expect(page.locator('.value-row.changed')).toHaveCount(1);
    await page.getByRole('button',{name:'Show action ↗',exact:true}).click();
    await page.getByRole('button',{name:'Generated TLA+',exact:true}).click();
    await page.getByRole('button',{name:'Open as editable TLA+ ↗',exact:true}).click();
    await run();
    await expect(page.getByText('Invariant NotSolved is violated.',{exact:true})).toBeVisible();
    await page.getByRole('button',{name:'TSL',exact:true}).click();
    await expect(page.locator('.cm-content')).toContainText('spec DieHard');
    await page.getByLabel('Example',{exact:true}).selectOption('3');
    await run();
    await expect(page.getByText('Configured checks passed for this finite model.',{exact:true})).toBeVisible();
    await expect(page.locator('svg g[role=button]')).toHaveCount(4);
    await page.getByRole('button',{name:'Select state 2',exact:true}).click();
    await expect(page.getByRole('heading',{name:'State S2',exact:true})).toBeVisible();
    await page.getByLabel('State limit',{exact:true}).fill('2');
    await page.getByLabel('State limit',{exact:true}).blur();
    await expect(page.getByText('Source or settings changed.',{exact:false})).toBeVisible();
    await run();
    await expect(page.getByText('State limit reached; verification is incomplete.',{exact:true})).toBeVisible();
    await page.getByLabel('State limit',{exact:true}).fill('500');
    await page.getByLabel('Example',{exact:true}).selectOption('6');
    await run();
    await expect(page.getByText('After the last state, return to state',{exact:false})).toBeVisible();
    await page.getByLabel('Example',{exact:true}).selectOption('0');
    await page.getByRole('button',{name:'TLA+',exact:true}).click();
    await run();
    await expect(page.getByText('Invariant NotSolved is violated.',{exact:true})).toBeVisible();
    await page.screenshot({path:'/tmp/tla-playground-tla.png'});
    await page.setViewportSize({width:390,height:844});
    await expect(page.getByRole('button',{name:'Trace',exact:true})).toBeVisible();
    await page.screenshot({path:'/tmp/tla-playground-mobile.png'});
    assert.deepEqual(errors,[]);
  }finally{await browser.close()}
});

test('inline errors, edits invalidate results, and stop ignores late responses',async()=>{
  const browser=await chromium.launch();
  try{
    const page=await browser.newPage();await page.goto(url);
    const editor=page.locator('.cm-content');
    await replaceSource(editor,'spec Broken { state x: Int = true; action stay {} }');
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    await expect(page.getByRole('button',{name:'Go to line 1'})).toBeVisible();
    await replaceSource(editor,'spec Good { state x: Int = 0; action stay {} }');
    await expect(page.getByText('Source or settings changed.',{exact:false})).toBeVisible();
    // Delay transport so Stop and revision handling are deterministic.
    await page.route(transport,async route=>{
      await new Promise(resolve=>setTimeout(resolve,700));
      try{await route.continue()}catch{}
    });
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    await page.getByRole('button',{name:'Stop',exact:true}).click();
    await page.waitForTimeout(900);
    await expect(page.getByRole('button',{name:'Run model →',exact:true})).toBeEnabled();
    await expect(page.getByText('Configured checks passed for this finite model.',{exact:true})).toHaveCount(0);
    await page.unroute(transport);
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    await expect(page.getByText('Configured checks passed for this finite model.',{exact:true})).toBeVisible();
  }finally{await browser.close()}
});

test('disconnect terminates an executing native checker process',{skip:browserMode},async()=>{
  const {execFileSync}=await import('node:child_process');
  const processes=()=>execFileSync('ps',['-axo','pid=,command='],{encoding:'utf8'}).split('\n')
    .filter(line=>/target\/debug\/tla-playground .*request\.json/.test(line));
  const before=new Set(processes());
  const controller=new AbortController();
  const request=fetch(url+'/api/check',{method:'POST',headers:{'Content-Type':'application/json'},signal:controller.signal,
    body:JSON.stringify({language:'tsl',source:'spec Slow { recursive fn work(n: Int) -> Int { if n == 0 { 1 } else { work(n - 1) + work(n - 1) } } state x: Int = work(27); action stay {} }',config:'',maxStates:500})}).catch(()=>null);
  try{
    let running=[];
    for(let i=0;i<20;i++){await new Promise(r=>setTimeout(r,50));running=processes().filter(line=>!before.has(line));if(running.length)break}
    assert.equal(running.length,1,'checker should be executing before cancellation');
    controller.abort();await request;
    for(let i=0;i<20&&processes().some(line=>running.includes(line));i++)await new Promise(r=>setTimeout(r,50));
    assert(!processes().some(line=>running.includes(line)),'cancelled worker must exit');
  }finally{controller.abort();await request}
});

test('all nine gallery puzzles run and render their selected state',async()=>{
  const browser=await chromium.launch({headless:true});
  try{
    const page=await browser.newPage({viewport:{width:1440,height:1000}});const errors=[];
    page.on('pageerror',e=>errors.push(e.message));await page.goto(url);
    for(const title of ['Knights and knaves','Feature selection','Mini Einstein','Cryptarithm','4×4 Sudoku','Bridge and torch','Warehouse robot','Staff rota','Mini factory']){
      await page.getByLabel('Example',{exact:true}).selectOption({label:title});
      await page.getByRole('button',{name:'Run model →',exact:true}).click();
      await expect(page.locator('.verdict')).toContainText(/Invariant (NoSolution|NotSolved) is violated/);
      const steps=page.locator('.trace-select');await steps.last().click();
      const projection=page.getByRole('region',{name:'Puzzle state view'});
      await expect(projection).toBeVisible();
      await expect(projection).not.toContainText('does not match');
      if(title==='4×4 Sudoku'){
        await expect(page.locator('.sudoku span')).toHaveCount(16);
        await expect(page.locator('.sudoku')).not.toContainText('·');
        await page.screenshot({path:'/tmp/tla-gallery-sudoku.png'});
      }
      if(title==='Bridge and torch')await expect(projection).toContainText('17 minutes');
      if(title==='Mini factory')await page.screenshot({path:'/tmp/tla-gallery-factory.png'});
    }
    // Export a gallery model and run its actual TLA+ in the other editor mode.
    await page.getByRole('button',{name:'Generated TLA+',exact:true}).click();
    await page.getByRole('button',{name:'Open as editable TLA+ ↗',exact:true}).click();
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    await expect(page.locator('.verdict')).toContainText('Invariant NoSolution is violated');
    assert.deepEqual(errors,[]);
  }finally{await browser.close()}
});

test('Paxos verifies agreement and visualizes the broken historical majorities',async()=>{
  const browser=await chromium.launch({headless:true});
  try{
    const page=await browser.newPage({viewport:{width:1440,height:1000}});
    const errors=[];page.on('pageerror',e=>errors.push(e.message));await page.goto(url);
    await page.getByLabel('Example',{exact:true}).selectOption({label:'Paxos · agreement'});
    await expect(page.getByLabel('State limit',{exact:true})).toHaveValue('2000');
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    await expect(page.locator('.verdict')).toContainText('Configured checks passed',{timeout:20000});
    await expect(page.getByRole('region',{name:'Paxos state view'})).toBeVisible();
    await page.getByLabel('Example',{exact:true}).selectOption({label:'Paxos · broken proposer'});
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    await expect(page.locator('.verdict')).toContainText('Invariant Agreement is violated',{timeout:20000});
    await expect(page.getByText('10 transitions',{exact:true})).toBeVisible();
    await page.locator('.trace-select').last().click();
    const view=page.getByRole('region',{name:'Paxos state view'});
    await expect(view).not.toContainText('does not match');
    await expect(view.locator('.proposals')).toContainText('Red');
    await expect(view.locator('.proposals')).toContainText('Blue');
    await expect(view.locator('.proposals span')).toHaveText(['2 historical acceptances','2 historical acceptances']);
    await page.screenshot({path:'/tmp/tla-paxos-playground.png'});
    assert.deepEqual(errors,[]);
  }finally{await browser.close()}
});

test('automatic checks debounce edits, cancel outdated runs, and preserve mobile editing',{skip:browserMode},async()=>{
  const browser=await chromium.launch();
  try{
    const page=await browser.newPage({viewport:{width:390,height:844}});
    const requests=[];page.on('request',request=>{if(request.url().endsWith('/api/check'))requests.push(request.postDataJSON())});
    await page.goto(url);
    await expect(page.getByRole('button',{name:/Results •/})).toBeVisible({timeout:10000});
    assert.equal(requests.length,1,'initial model runs without a click');
    const editor=page.getByRole('textbox',{name:'Model source'});
    await replaceSource(editor,'spec Auto { state x: Int = 1; action stay {} }');
    await page.waitForTimeout(250);
    await replaceSource(editor,'spec Auto { state x: Int = 2; action stay {} }');
    await page.waitForTimeout(250);
    assert.equal(requests.length,1,'typing has not triggered a check yet');
    await expect.poll(()=>requests.length).toBe(2);
    assert(requests[1].source.includes('= 2;'),'only the latest draft is checked');
    await expect(editor).toBeVisible();
    await page.getByRole('button',{name:/Results/}).click();
    await expect(page.locator('.verdict')).toContainText('Configured checks passed');
    await page.getByRole('button',{name:'Source',exact:true}).click();
    // An obsolete response must never overwrite a newer model, even if transport is delayed.
    let release;const held=new Promise(resolve=>release=resolve);
    await page.route('**/api/check',async route=>{
      if(route.request().postDataJSON().source.includes('Obsolete')){
        await held;try{await route.fulfill({json:{status:'error',message:'OBSOLETE RESPONSE'}})}catch{}
      }else await route.continue();
    });
    await replaceSource(editor,'spec Obsolete { state x: Int = 0; action stay {} }');
    await expect.poll(()=>requests.length).toBe(3);
    await replaceSource(editor,'spec Latest { state x: Int = 3; action stay {} }');
    release();
    await expect.poll(()=>requests.length).toBe(4);
    await page.getByRole('button',{name:/Results/}).click();
    await expect(page.locator('.verdict')).toContainText('Configured checks passed');
    await expect(page.getByText('OBSOLETE RESPONSE')).toHaveCount(0);
    await page.getByRole('button',{name:'Source',exact:true}).click();
    await page.getByRole('button',{name:'● Auto',exact:true}).click();
    await replaceSource(editor,'spec Paused { state x: Int = 4; action stay {} }');
    await page.waitForTimeout(1000);assert.equal(requests.length,4);
    await page.getByRole('button',{name:'○ Paused',exact:true}).click();
    await expect.poll(()=>requests.length).toBe(5);
    await page.screenshot({path:'/tmp/tla-studio-mobile.png'});
  }finally{await browser.close()}
});

test('searchable library, editor controls, and highlighted generated source',async()=>{
  const browser=await chromium.launch();
  try{
    const page=await browser.newPage();await page.goto(url);
    await page.getByRole('button',{name:/Examples/}).click();
    await page.getByRole('textbox',{name:'Search examples'}).fill('counter');
    await page.locator('.library-list button').filter({has:page.getByText('Counter',{exact:true})}).click();
    await expect(page.locator('dialog')).not.toBeVisible();
    await expect(page.locator('.verdict')).toContainText('Configured checks passed');
    const content=page.getByRole('textbox',{name:'Model source'});
    const colors=await content.locator('span').evaluateAll(nodes=>[...new Set(nodes.map(n=>getComputedStyle(n).color))]);
    assert(colors.length>=4,'keywords, types, numbers, and identifiers have distinct colors');
    await page.getByRole('button',{name:'Wrap',exact:true}).click();
    await expect(page.locator('.source-pane .cm-content')).toHaveClass(/cm-lineWrapping/);
    const before=await content.evaluate(n=>getComputedStyle(n).fontSize);
    await page.getByRole('button',{name:'Increase editor font size'}).click();
    assert.notEqual(await content.evaluate(n=>getComputedStyle(n).fontSize),before);
    await page.getByRole('button',{name:'Generated TLA+',exact:true}).click();
    await expect(page.locator('.generated-editor .cm-content')).toHaveAttribute('contenteditable','false');
    assert(await page.locator('.generated-editor .cm-line span').count()>5,'generated code is highlighted');
  }finally{await browser.close()}
});


test('browser checks stay local and terminate on Stop and timeout',{skip:!browserMode},async()=>{
  const browser=await chromium.launch();
  try {
    const page=await browser.newPage();
    const api=[];
    page.on('request',r=>{if(r.url().includes('/api/'))api.push(r.url())});
    await page.goto(url);
    await expect(page.locator('.verdict')).toContainText('Invariant NotSolved is violated.');
    await page.getByRole('button',{name:'● Auto',exact:true}).click();
    const editor=page.getByRole('textbox',{name:'Model source'});
    await replaceSource(editor,'spec Slow { recursive fn work(n: Int) -> Int { if n == 0 { 1 } else { work(n - 1) + work(n - 1) } } state x: Int = work(40); action stay {} }');
    const started=page.waitForEvent('worker');
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    const worker=await started;
    const closed=worker.waitForEvent('close');
    await page.getByRole('button',{name:'Stop',exact:true}).click();
    await closed;
    await expect(page.getByRole('button',{name:'Run model →',exact:true})).toBeEnabled();
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    await expect(page.locator('.verdict')).toContainText('15-second time limit reached.',{timeout:20000});
    await replaceSource(editor,'spec Good { state x: Int = 0; action stay {} }');
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    await expect(page.locator('.verdict')).toContainText('Configured checks passed');
    assert.deepEqual(api,[],'browser checking never sends model source to an API');
  } finally {await browser.close()}
});

test('dense traces link exact states and large graphs retain edge navigation',async()=>{
  const browser=await chromium.launch();
  try {
    const page=await browser.newPage();await page.goto(url);
    await expect(page.locator('.trace-table tbody tr')).toHaveCount(7);
    await page.getByRole('button',{name:'Select trace step 1',exact:true}).click();
    await expect(page.locator('.trace-table tr.selected td.delta')).toHaveCount(1);
    await page.getByLabel('Changed only').check();
    await expect(page.locator('.value-row')).toHaveCount(1);
    await page.getByRole('button',{name:/Graph S\d+ ↗/}).click();
    await expect(page.getByRole('table',{name:'State transitions',exact:true})).toBeVisible();
    await expect(page.locator('.comparison')).toContainText('discovery parent');
    await page.getByRole('button',{name:'● Auto',exact:true}).click();
    const editor=page.getByRole('textbox',{name:'Model source'});
    await replaceSource(editor,'spec Many { state x: Int = 0; action increment { require x < 299; next x = x + 1; } action stay {} }');
    await page.getByRole('button',{name:'Run model →',exact:true}).click();
    await expect(page.locator('.verdict')).toContainText('Configured checks passed');
    await expect(page.getByLabel('Graph scope')).toHaveValue('neighbors');
    await expect(page.locator('.graph-navigation')).toContainText('2/300 states');
    await page.getByLabel('Find state ID').fill('299');await page.getByRole('button',{name:'Go',exact:true}).click();
    await expect(page.getByRole('button',{name:'Select state 299',exact:true})).toBeVisible();
    await expect(page.getByRole('heading',{name:'State S299',exact:true})).toBeVisible();
    await page.getByRole('button',{name:'Outgoing 1',exact:true}).click();
    await page.getByRole('button',{name:'→ S299 ↶',exact:true}).click();
    await expect(page.locator('.comparison')).toContainText('0/1 changed · vs edge source S299');
    await page.getByRole('button',{name:'Incoming 2',exact:true}).click();
    const edge=page.getByRole('table',{name:'State transitions',exact:true}).locator('tbody tr').filter({hasText:'increment'});
    await edge.getByRole('button',{name:'→ S299',exact:true}).click();
    await expect(page.locator('.comparison')).toContainText('1/1 changed · vs edge source S298');
    await page.getByLabel('Graph scope').selectOption('overview');
    await expect(page.locator('.graph-navigation')).toContainText('50/300 states');
    await page.getByRole('button',{name:'← 250',exact:true}).click();
    await expect(page.locator('.graph-navigation')).toContainText('250/300 states');
    await expect(page.locator('svg g[role=button]')).toHaveCount(250);
  }finally{await browser.close()}
});
