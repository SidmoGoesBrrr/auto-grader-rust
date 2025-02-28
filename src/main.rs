// Import Rocket macros and external crates.
#[macro_use] extern crate rocket;

use rocket::form::Form;
use rocket::fs::{TempFile, FileServer, relative};
use rocket::response::content::RawHtml;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::io::Write;
use std::fs;
use std::env;
use std::path::{Path, PathBuf};

/// Data structure representing a single test case.
/// Each test case includes a description, input string, and expected output.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct TestCase {
    description: String,
    input: String,
    expected_output: String,
}

/// Type alias for a mapping from question IDs to lists of test cases.
type TestCasesMap = HashMap<String, Vec<TestCase>>;

//
// Routes for Uploading and Testing Code
//

/// Form data structure for file uploads.
/// Includes the uploaded C file and the question identifier.
#[derive(FromForm)]
struct Upload<'r> {
    file: TempFile<'r>,
    question: String,
}

/// GET /
/// Returns the index page with a form for uploading C code.
/// The page also includes a link to the favicon (served from /static).
#[get("/")]
async fn index() -> RawHtml<&'static str> {
    RawHtml(r#"
    <!DOCTYPE html>
    <html>
      <head>
        <meta charset="UTF-8">
        <title>Autograder</title>
        <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
        <link rel="icon" href="/static/favicon.ico" type="image/x-icon">
      </head>
      <body>
        <div class="container mt-5">
          <h1>Upload Your C Code</h1>
          <form action="/upload" method="post" enctype="multipart/form-data">
            <div class="mb-3">
              <label for="question" class="form-label">Select Question:</label>
              <select id="question" name="question" class="form-select">
                <option value="q1">Q1</option>
                <option value="q2">Q2</option>
                <option value="q3">Q3</option>
              </select>
            </div>
            <div class="mb-3">
              <label for="file" class="form-label">C File:</label>
              <input type="file" class="form-control" id="file" name="file" accept=".c">
            </div>
            <button type="submit" class="btn btn-primary">Submit</button>
          </form>
        </div>
      </body>
    </html>
    "#)
}

/// POST /upload
/// Handles the file upload, compiles the C code, loads test cases, and runs them
/// inside a sandboxed environment using NSJail.
#[post("/upload", data = "<form>")]
async fn upload(mut form: Form<Upload<'_>>) -> RawHtml<String> {
    use uuid::Uuid;

    // Check if a file was uploaded.
    if form.file.name().is_none() {
        return RawHtml("<h2>No file uploaded. Try again with a valid .c file</h2>".to_string());
    }

    // Build an absolute path for the temporary directory.
    let cwd = env::current_dir().expect("Failed to get current directory");
    let temp_dir: PathBuf = cwd.join("tempfiles");

    // Create the tempfiles directory if it doesn't exist.
    if let Err(e) = fs::create_dir_all(&temp_dir) {
        return RawHtml(format!("<h2>Error creating temp directory: {}</h2>", e));
    }

    // Generate unique file names using Uuid.
    let unique_id = Uuid::new_v4().to_string();
    let tmp_path = temp_dir.join(format!("{}.c", unique_id));
    let exe_path = temp_dir.join(&unique_id);

    // Save the uploaded C file to disk.
    if let Err(e) = form.file.persist_to(&tmp_path).await {
        return RawHtml(format!("<h2>Error saving file: {}</h2>", e));
    }

    // Compile the C file using gcc.
    let compile = Command::new("gcc")
        .arg("-o")
        .arg(&exe_path)
        .arg(&tmp_path)
        .output();

    // Handle compilation errors.
    let compile_output = match compile {
        Ok(output) => output,
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            return RawHtml(format!("<h2>Compilation failed: {}</h2>", e));
        }
    };

    if !compile_output.status.success() {
        let err_msg = String::from_utf8_lossy(&compile_output.stderr);
        let _ = fs::remove_file(&tmp_path);
        return RawHtml(format!("<h2>Compilation errors:</h2><pre>{}</pre>", err_msg));
    }

    // Set executable permissions explicitly.
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = fs::set_permissions(&exe_path, fs::Permissions::from_mode(0o755)) {
        eprintln!("Error setting permissions on executable: {}", e);
    }

    // Verify that the executable exists.
    let exe_path_str = exe_path.to_string_lossy().into_owned();
    if !Path::new(&exe_path).exists() {
        eprintln!("Executable not found at: {}", exe_path_str);
        return RawHtml("<h2>Internal error: compiled executable not found.</h2>".to_string());
    }

    // Load test cases from the external JSON file.
    let test_cases_data = match fs::read_to_string("test_cases.json") {
        Ok(data) => data,
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            let _ = fs::remove_file(&exe_path);
            return RawHtml(format!("<h2>Error reading test cases file: {}</h2>", e));
        }
    };

    let test_cases_map: TestCasesMap = match serde_json::from_str(&test_cases_data) {
        Ok(tc) => tc,
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            let _ = fs::remove_file(&exe_path);
            return RawHtml(format!("<h2>Error parsing test cases file: {}</h2>", e));
        }
    };

    let selected_question = &form.question;
    let cases = match test_cases_map.get(selected_question) {
        Some(c) => c,
        None => {
            let _ = fs::remove_file(&tmp_path);
            let _ = fs::remove_file(&exe_path);
            return RawHtml(format!("<h2>No test cases found for question {}</h2>", selected_question));
        }
    };

    let mut results = Vec::new();

    // Loop through each test case.
    for case in cases {
        // Check again that the executable exists.
        if !Path::new(&exe_path).exists() {
            eprintln!("Executable not found at: {}", exe_path_str);
            return RawHtml("<h2>Internal error: compiled executable not found.</h2>".to_string());
        }

        // Use NSJail to run the executable. We bindmount the tempfiles directory,
        // as well as necessary system directories for dynamic linking.
        let mut child = match Command::new("nsjail")
            .args(&[
                "--mode=exec",
                "--disable_clone_newuser",
                // Bind the tempfiles directory so the executable is visible.
                "--bindmount", "/app/tempfiles:/app/tempfiles",
                // Bind additional system library directories (adjust these as needed):
                "--bindmount", "/lib:/lib",
                "--bindmount", "/usr/lib:/usr/lib",
                "--", &exe_path_str,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn() {
                Ok(child) => child,
                Err(e) => {
                    results.push((case.description.clone(), false, format!("Error running the program with NSJail: {}", e)));
                    continue;
                }
            };

        // Write the test case input to the program's stdin.
        {
            let child_stdin = child.stdin.as_mut().expect("Failed to open stdin");
            if let Err(e) = child_stdin.write_all(case.input.as_bytes()) {
                results.push((case.description.clone(), false, format!("Error writing to stdin: {}", e)));
                continue;
            }
        }

        // Wait for the program to finish and capture its output.
        let run_output = match child.wait_with_output() {
            Ok(output) => output,
            Err(e) => {
                results.push((case.description.clone(), false, format!("Error waiting for output: {}", e)));
                continue;
            }
        };

        let actual_output = String::from_utf8_lossy(&run_output.stdout).trim().to_string();
        let expected_substring = case.expected_output.trim();
        let passed = actual_output.contains(expected_substring);
        let result_text = format!("Input: {}\nExpected to contain: {}\nGot: {}", case.input, expected_substring, actual_output);
        results.push((case.description.clone(), passed, result_text));
    }

    // Clean up temporary files after processing all test cases.
    let _ = fs::remove_file(&tmp_path);
    let _ = fs::remove_file(&exe_path);

    // Build the HTML output to display test results with an animated reveal.
    let mut results_html = String::from("<h1>Test Results</h1><div id='results'>");
    for (i, (desc, passed, details)) in results.into_iter().enumerate() {
        let bg_class = if passed { "bg-success" } else { "bg-danger" };
        results_html.push_str(&format!(
            "<div class='list-group-item {} text-white test-result' style='display:none;' data-delay='{}'>
              <strong>{}</strong>: {}
              <pre>{}</pre>
            </div>", 
            bg_class, i * 500, desc, if passed { "Passed" } else { "Failed" }, details
        ));
    }
    results_html.push_str("</div><a href='/' class='btn btn-secondary'>Upload another file</a>");

    let script = r#"
    <script>
      window.addEventListener('DOMContentLoaded', () => {
        const results = document.querySelectorAll('.test-result');
        results.forEach((result, index) => {
          setTimeout(() => {
            result.style.display = 'block';
            result.style.opacity = 0;
            let op = 0;
            const timer = setInterval(() => {
              if (op >= 1) clearInterval(timer);
              result.style.opacity = op;
              op += 0.1;
            }, 30);
          }, index * 500);
        });
      });
    </script>
    "#;
    let full_html = format!(r#"
    <!DOCTYPE html>
    <html>
      <head>
        <meta charset="UTF-8">
        <title>Execution Result</title>
        <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
      </head>
      <body>
        <div class="container mt-5">
          {}
        </div>
        {}
      </body>
    </html>
    "#, results_html, script);

    RawHtml(full_html)
}

//
// Admin Panel Routes
//

/// GET /admin
/// Returns the admin login page.
#[get("/admin")]
async fn admin_login_page() -> RawHtml<String> {
    RawHtml(r#"
    <!DOCTYPE html>
    <html>
      <head>
        <meta charset="UTF-8">
        <title>Admin Login</title>
        <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
      </head>
      <body>
        <div class="container mt-5">
          <h1>Admin Login</h1>
          <form action="/admin" method="post">
            <div class="mb-3">
              <label for="password" class="form-label">Password:</label>
              <input type="password" id="password" name="password" class="form-control">
            </div>
            <button type="submit" class="btn btn-primary">Login</button>
          </form>
        </div>
      </body>
    </html>
    "#.to_string())
}

/// Data structure representing admin login credentials.
#[derive(rocket::form::FromForm)]
struct AdminLogin {
    password: String,
}

/// POST /admin
/// Processes the admin login and, if successful, displays the admin panel.
#[post("/admin", data = "<form>")]
async fn admin_login(form: Form<AdminLogin>) -> RawHtml<String> {
    let admin_password = "secret";
    if form.password != admin_password {
        return RawHtml("<h2>Invalid password.</h2><a href='/admin'>Try again</a>".to_string());
    }
    let html = r#"
       <!DOCTYPE html>
       <html>
         <head>
           <meta charset="UTF-8">
           <title>Admin Panel</title>
           <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
         </head>
         <body>
           <div class="container mt-5">
             <h1>Admin Panel - Select Question to Edit Test Cases</h1>
             <ul class="list-group">
               <li class="list-group-item"><a href="/admin/edit?question=q1">Edit Q1 Test Cases</a></li>
               <li class="list-group-item"><a href="/admin/edit?question=q2">Edit Q2 Test Cases</a></li>
               <li class="list-group-item"><a href="/admin/edit?question=q3">Edit Q3 Test Cases</a></li>
             </ul>
           </div>
         </body>
       </html>
    "#;
    RawHtml(html.to_string())
}

/// GET /admin/edit
/// Returns a page for editing test cases for a given question.
#[get("/admin/edit?<question>")]
async fn admin_edit_page(question: Option<String>) -> RawHtml<String> {
    let q = question.unwrap_or_else(|| "q1".to_string());
    let content = fs::read_to_string("test_cases.json").unwrap_or_else(|_| "{}".to_string());
    let mut test_cases_map: TestCasesMap = serde_json::from_str(&content).unwrap_or_else(|_| HashMap::new());
    let cases = test_cases_map.entry(q.clone()).or_insert(Vec::new());

    let mut form_html = format!(r#"
       <!DOCTYPE html>
       <html>
         <head>
           <meta charset="UTF-8">
           <title>Edit Test Cases for {}</title>
           <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
           <style>
             .test-case {{
               margin-bottom: 20px; 
               padding: 10px; 
               border: 1px solid #ccc; 
               position: relative;
             }}
             .remove-btn {{
               position: absolute;
               top: 10px;
               right: 10px;
             }}
           </style>
         </head>
         <body>
           <div class="container mt-5">
             <h1>Edit Test Cases for {}</h1>
             <form id="test-cases-form" action="/admin/edit" method="post">
               <input type="hidden" name="question" value="{}">
    "#, q, q, q);

    for case in cases {
        form_html.push_str(&format!(r#"
           <div class="test-case">
             <button type="button" class="btn btn-danger btn-sm remove-btn" onclick="removeTestCase(this)">X</button>
             <div class="mb-3">
               <label>Description:</label>
               <input type="text" class="form-control" name="desc" value="{}">
             </div>
             <div class="mb-3">
               <label>Input:</label>
               <textarea class="form-control" name="inp" rows="3">{}</textarea>
             </div>
             <div class="mb-3">
               <label>Expected Output:</label>
               <textarea class="form-control" name="exp" rows="3">{}</textarea>
             </div>
           </div>
        "#,
        htmlescape::encode_minimal(&case.description),
        htmlescape::encode_minimal(&case.input),
        htmlescape::encode_minimal(&case.expected_output)));
    }

    form_html.push_str(r#"
               <div id="new-test-case-container"></div>
               <button type="button" class="btn btn-secondary" onclick="addTestCase()">Add Test Case</button>
               <button type="submit" class="btn btn-primary">Save Changes</button>
             </form>
             <a href="/admin" class="btn btn-secondary mt-3">Back to Admin Panel</a>
           </div>
           <script>
             function removeTestCase(button) {
               button.parentElement.remove();
             }
             function addTestCase() {
               const container = document.getElementById("new-test-case-container");
               const div = document.createElement("div");
               div.className = "test-case";
               div.innerHTML = `
                 <button type="button" class="btn btn-danger btn-sm remove-btn" onclick="removeTestCase(this)">Remove</button>
                 <div class="mb-3">
                   <label>Description:</label>
                   <input type="text" class="form-control" name="desc" value="">
                 </div>
                 <div class="mb-3">
                   <label>Input:</label>
                   <textarea class="form-control" name="inp" rows="3"></textarea>
                 </div>
                 <div class="mb-3">
                   <label>Expected Output:</label>
                   <textarea class="form-control" name="exp" rows="3"></textarea>
                 </div>
               `;
               container.appendChild(div);
             }
           </script>
         </body>
       </html>
    "#);
    RawHtml(form_html)
}

/// Data structure for the form used to update test cases.
#[derive(rocket::form::FromForm)]
struct AdminEditForm {
    question: String,
    desc: Vec<String>,
    inp: Vec<String>,
    exp: Vec<String>,
}

/// POST /admin/edit
/// Updates the test cases for a given question and writes them back to the JSON file.
#[post("/admin/edit", data = "<form>")]
async fn admin_edit_update(form: Form<AdminEditForm>) -> RawHtml<String> {
    let q = &form.question;
    let content = fs::read_to_string("test_cases.json").unwrap_or_else(|_| "{}".to_string());
    let mut test_cases_map: TestCasesMap = serde_json::from_str(&content).unwrap_or_else(|_| HashMap::new());
    let mut new_cases = Vec::new();
    let n = form.desc.len().min(form.inp.len()).min(form.exp.len());
    for i in 0..n {
        new_cases.push(TestCase {
            description: form.desc[i].clone(),
            input: form.inp[i].clone(),
            expected_output: form.exp[i].clone(),
        });
    }
    test_cases_map.insert(q.clone(), new_cases);
    let new_content = serde_json::to_string_pretty(&test_cases_map).unwrap_or_else(|_| "{}".to_string());
    if let Err(e) = fs::write("test_cases.json", new_content) {
        return RawHtml(format!("<h2>Error updating test cases: {}</h2>", e));
    }
    
    RawHtml(format!("<h2>Test cases for {} updated successfully.</h2><a href='/admin'>Back to Admin Panel</a>", q))
}

//
// Launch the Application
//

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", routes![
            index, 
            upload, 
            admin_login_page, 
            admin_login, 
            admin_edit_page, 
            admin_edit_update
        ])
        .mount("/static", FileServer::from(relative!("static")))
}
