#[macro_use] extern crate rocket;

use rocket::form::Form;
use rocket::fs::TempFile;
use rocket::response::content::RawHtml;
use serde::Deserialize;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::io::Write;
use std::fs;

#[derive(FromForm)]
struct Upload<'r> {
    file: TempFile<'r>,
    question: String,
}

#[derive(Deserialize)]
struct TestCase {
    description: String,
    input: String,
    expected_output: String,
}

type TestCases = HashMap<String, Vec<TestCase>>;

#[get("/")]
async fn index() -> RawHtml<&'static str> {
    RawHtml(r#"
    <!DOCTYPE html>
    <html>
      <head>
        <meta charset="UTF-8">
        <title>Autograder</title>
        <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
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

#[post("/upload", data = "<form>")]
async fn upload(mut form: Form<Upload<'_>>) -> RawHtml<String> {
    use uuid::Uuid;

    // Define the directory for temporary files.
    let temp_dir = "tempfiles";

    // Ensure the directory exists.
    if let Err(e) = fs::create_dir_all(temp_dir) {
        return RawHtml(format!("<h2>Error creating temp directory: {}</h2>", e));
    }

    // Generate a unique ID and create file paths.
    let unique_id = Uuid::new_v4();
    let tmp_path = format!("{}/{}.c", temp_dir, unique_id);
    let exe_path = format!("{}/{}", temp_dir, unique_id);

    // Save the uploaded file to the unique temporary location.
    if let Err(e) = form.file.persist_to(&tmp_path).await {
        return RawHtml(format!("<h2>Error saving file: {}</h2>", e));
    }

    // Compile the C file using gcc.
    let compile = Command::new("gcc")
        .arg("-o")
        .arg(&exe_path)
        .arg(&tmp_path)
        .output();

    let compile_output = match compile {
        Ok(output) => output,
        Err(e) => return RawHtml(format!("<h2>Compilation failed: {}</h2>", e)),
    };

    if !compile_output.status.success() {
        let err_msg = String::from_utf8_lossy(&compile_output.stderr);
        return RawHtml(format!("<h2>Compilation errors:</h2><pre>{}</pre>", err_msg));
    }

    // Load test cases from the JSON file.
    let test_cases_data = match fs::read_to_string("test_cases.json") {
        Ok(data) => data,
        Err(e) => return RawHtml(format!("<h2>Error reading test cases file: {}</h2>", e)),
    };

    let test_cases: TestCases = match serde_json::from_str(&test_cases_data) {
        Ok(tc) => tc,
        Err(e) => return RawHtml(format!("<h2>Error parsing test cases file: {}</h2>", e)),
    };

    // Retrieve the test cases for the selected question.
    let selected_question = &form.question;
    let cases = match test_cases.get(selected_question) {
        Some(c) => c,
        None => return RawHtml(format!("<h2>No test cases found for question {}</h2>", selected_question)),
    };

    // Run each test case.
    let mut results = Vec::new();

    for case in cases {
        // Spawn the compiled program and pass the test case input.
        let mut child = match Command::new(format!("./{}", exe_path))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn() {
                Ok(child) => child,
                Err(e) => {
                    results.push(format!("Test '{}' - Error running the program: {}", case.description, e));
                    continue;
                }
        };

        {
            let child_stdin = child.stdin.as_mut().expect("Failed to open stdin");
            if let Err(e) = child_stdin.write_all(case.input.as_bytes()) {
                results.push(format!("Test '{}' - Error writing to stdin: {}", case.description, e));
                continue;
            }
        }

        let run_output = match child.wait_with_output() {
            Ok(output) => output,
            Err(e) => {
                results.push(format!("Test '{}' - Error waiting for output: {}", case.description, e));
                continue;
            }
        };

        let actual_output = String::from_utf8_lossy(&run_output.stdout).trim().to_string();
        let expected_substring = case.expected_output.trim();

        let status = if actual_output.contains(expected_substring) {
            "Passed"
        } else {
            "Failed"
        };

        results.push(format!(
            "Test '{}': {}\nInput: {}\nExpected to contain: {}\nGot: {}\n",
            case.description, status, case.input, expected_substring, actual_output
        ));
    }

    // Build the results HTML.
    let mut results_html = String::from("<h1>Test Results</h1><div class='list-group'>");
    for res in results {
        results_html.push_str(&format!("<pre class='list-group-item'>{}</pre>", res));
    }
    results_html.push_str("<a href='/' class='btn btn-secondary'>Upload another file</a>");
    // Remove temp files
    fs::remove_file(&tmp_path).expect("Failed to remove temp file");
    fs::remove_file(&exe_path).expect("Failed to remove temp file");

    RawHtml(format!(r#"
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
      </body>
    </html>
    "#, results_html))
}

// Admin Panel Routes
//

// GET /admin shows a simple password login form.
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

#[derive(rocket::form::FromForm)]
struct AdminLogin {
    password: String,
}

// POST /admin processes the login and, if successful, shows the JSON editor.
#[post("/admin", data = "<form>")]
async fn admin_login(form: Form<AdminLogin>) -> RawHtml<String> {
    // Hard-coded admin password for this example.
    let admin_password = "secret";
    if form.password != admin_password {
        return RawHtml("<h2>Invalid password.</h2><a href='/admin'>Try again</a>".to_string());
    }
    // Read the current test cases JSON.
    let content = fs::read_to_string("test_cases.json").unwrap_or_else(|_| "{}".to_string());
    let html = format!(r#"
       <!DOCTYPE html>
       <html>
         <head>
           <meta charset="UTF-8">
           <title>Admin Panel</title>
           <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
         </head>
         <body>
           <div class="container mt-5">
             <h1>Admin Panel - Edit Test Cases</h1>
             <form action="/admin/update" method="post">
               <div class="mb-3">
                 <label for="json_content" class="form-label">Test Cases JSON</label>
                 <textarea id="json_content" name="json_content" class="form-control" rows="20">{}</textarea>
               </div>
               <button type="submit" class="btn btn-primary">Update Test Cases</button>
             </form>
           </div>
         </body>
       </html>
       "#, content);
    RawHtml(html)
}

#[derive(rocket::form::FromForm)]
struct AdminUpdate {
    json_content: String,
}

// POST /admin/update accepts updated JSON content and writes it to test_cases.json.
#[post("/admin/update", data = "<form>")]
async fn admin_update(form: Form<AdminUpdate>) -> RawHtml<String> {
    if let Err(e) = fs::write("test_cases.json", &form.json_content) {
        return RawHtml(format!("<h2>Error updating test cases: {}</h2>", e));
    }
    RawHtml("<h2>Test cases updated successfully.</h2><a href='/admin'>Back to Admin</a>".to_string())
}

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![index, upload,admin_login_page, 
    admin_login, 
    admin_update])
}
