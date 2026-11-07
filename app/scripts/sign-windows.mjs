// Signs one Windows file (the app or an installer). Tauri calls this for every file it builds, through the config the
// workflow writes (bundle.windows.signCommand). The certificate comes from GitHub secrets; with none set, the file is
// left unsigned and the build carries on.
//
//   WINDOWS_PFX_BASE64 + WINDOWS_PFX_PASSWORD   a .pfx certificate (an OV/EV certificate exported with its key, or
//                                               the throwaway test certificate the workflow can make)
//   ESIGNER_USERNAME, ESIGNER_PASSWORD,         SSL.com eSigner cloud signing (OV certificates keep their key in
//   ESIGNER_CREDENTIAL_ID, ESIGNER_TOTP_SECRET  the cloud since 2023); CODESIGNTOOL_DIR points at CodeSignTool,
//                                               which the workflow downloads (its Windows package includes Java)
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdtempSync, readdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";

const file = process.argv[2];
if (!file) { console.error("sign-windows: no file given"); process.exit(1); }
const env = process.env;
const TIMESTAMP = env.WINDOWS_TIMESTAMP_URL || "http://timestamp.digicert.com";

function signtool() {
  const root = "C:\\Program Files (x86)\\Windows Kits\\10\\bin";
  const versions = existsSync(root) ? readdirSync(root).filter((d) => /^10\./.test(d)).sort().reverse() : [];
  for (const v of versions) { const p = join(root, v, "x64", "signtool.exe"); if (existsSync(p)) return p; }
  return "signtool.exe";
}

if (env.WINDOWS_PFX_BASE64) {
  const dir = mkdtempSync(join(tmpdir(), "sign-"));
  const pfx = join(dir, "cert.pfx");
  try {
    writeFileSync(pfx, Buffer.from(env.WINDOWS_PFX_BASE64, "base64"));
    execFileSync(signtool(), ["sign", "/fd", "sha256", "/f", pfx, "/p", env.WINDOWS_PFX_PASSWORD ?? "", "/tr", TIMESTAMP, "/td", "sha256",
      "/d", "Wildlife Network Studio", "/du", "https://wlnw.org", file], { stdio: "inherit" });
    console.log(`sign-windows: signed ${file} with the .pfx certificate`);
  } finally { rmSync(dir, { recursive: true, force: true }); }
} else if (env.ESIGNER_USERNAME && env.ESIGNER_PASSWORD) {
  // CodeSignTool writes the signed copy to an output folder (overwriting in place can stop to ask), then we move it back.
  const toolDir = env.CODESIGNTOOL_DIR;
  if (!toolDir || !existsSync(join(toolDir, "CodeSignTool.bat"))) { console.error("sign-windows: CODESIGNTOOL_DIR is not set up"); process.exit(1); }
  const out = mkdtempSync(join(tmpdir(), "esigner-"));
  // CodeSignTool picks the format from the extension; Tauri hands over the NSIS uninstaller as a .tmp file.
  const known = /\.(exe|dll|msi|sys|cab|ps1|appx|msix)$/i.test(file);
  const input = known ? file : join(mkdtempSync(join(tmpdir(), "esigner-in-")), "uninstall.exe");
  if (!known) copyFileSync(file, input);
  const args = ["/c", "CodeSignTool.bat", "sign", `-username=${env.ESIGNER_USERNAME}`, `-password=${env.ESIGNER_PASSWORD}`,
    `-input_file_path=${input}`, `-output_dir_path=${out}`, "-program_name=Wildlife Network Studio"];
  if (env.ESIGNER_CREDENTIAL_ID) args.push(`-credential_id=${env.ESIGNER_CREDENTIAL_ID}`);
  if (env.ESIGNER_TOTP_SECRET) args.push(`-totp_secret=${env.ESIGNER_TOTP_SECRET}`);
  try {
    // The Java runtime inside CodeSignTool may not trust SSL.com's certificate chain: use the Windows certificate store.
    const javaEnv = { ...env, JAVA_TOOL_OPTIONS: `${env.JAVA_TOOL_OPTIONS ?? ""} -Djavax.net.ssl.trustStoreType=Windows-ROOT`.trim() };
    const log = execFileSync("cmd.exe", args, { cwd: toolDir, encoding: "utf8", env: javaEnv });
    process.stdout.write(log.replace(/-password=\S+/g, "-password=***"));
    const signed = join(out, basename(input));
    if (!existsSync(signed) || /error/i.test(log)) { console.error("sign-windows: eSigner did not return a signed file"); process.exit(1); }
    copyFileSync(signed, file);
    console.log(`sign-windows: signed ${file} with SSL.com eSigner`);
  } finally { rmSync(out, { recursive: true, force: true }); }
} else {
  console.log(`sign-windows: no certificate configured, ${file} stays unsigned`);
}
